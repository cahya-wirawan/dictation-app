//! `dictation-bench`: release evidence tooling (docs/08 §4–§6).
//!
//! ```text
//! dictation-bench corpus   --worker W --model M --sha S --manifest corpus.tsv [--trials 30] [--cold 10] [--threads N] [--json out.json]
//! dictation-bench nospeech [--seconds 8] [--seeds 5] [--dir DIR_OF_WAVS] [--json out.json]
//! dictation-bench soak     --worker W --model M --sha S [--sessions 500] [--kill-every 10] [--json out.json]
//! ```
//!
//! `corpus.tsv`: one utterance per line, `path<TAB>language<TAB>reference`,
//! paths relative to the manifest; `#` starts a comment. Audio may be any
//! WAV (i16/f32, mono/stereo, any rate); it is converted with the app's own
//! downmix + resampler. Output is a Markdown report on stdout and optional
//! JSON. Transcripts are printed only for the corpus (consented test data),
//! never by `soak`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use dictation_audio::convert::downmix_into;
use dictation_audio::detector::{DetectorConfig, EarshotDetector};
use dictation_audio::endpoint::{EndpointConfig, Outcome as EpOutcome};
use dictation_audio::pipeline::Pipeline;
use dictation_audio::resample::StreamResampler;
use dictation_bench::{Counts, cer, nospeech, stats, upper_95, wer, wer_raw};
use dictation_domain::SessionId;
use serde_json::json;
use transcription_protocol::client::WorkerClient;
use transcription_protocol::{JobId, Language};

type R<T> = Result<T, String>;

struct Args(Vec<String>);
impl Args {
    fn get(&self, k: &str) -> Option<&str> {
        self.0
            .iter()
            .position(|a| a == k)
            .and_then(|i| self.0.get(i + 1))
            .map(String::as_str)
    }
    fn req(&self, k: &str) -> R<&str> {
        self.get(k).ok_or_else(|| format!("missing {k}"))
    }
    fn num(&self, k: &str, default: usize) -> R<usize> {
        self.get(k)
            .map_or(Ok(default), |v| v.parse().map_err(|_| format!("bad {k}")))
    }
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        eprintln!("usage: dictation-bench <corpus|nospeech|soak> …");
        return ExitCode::from(2);
    };
    let a = Args(argv);
    let r = match cmd.as_str() {
        "corpus" => corpus(&a),
        "nospeech" => no_speech(&a),
        "soak" => soak(&a),
        _ => Err(format!("unknown command {cmd}")),
    };
    match r {
        Ok(report) => {
            if let Some(p) = a.get("--json") {
                let s = serde_json::to_string_pretty(&report).unwrap_or_default();
                if let Err(e) = std::fs::write(p, s) {
                    eprintln!("cannot write {p}: {e}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

// ---------- audio ----------

/// Any WAV → 16 kHz mono f32 via the app's own conversion path.
fn load_wav(p: &Path) -> R<Vec<f32>> {
    let mut r = hound::WavReader::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let spec = r.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>(),
        hound::SampleFormat::Int => {
            let scale = 2f32.powi(i32::from(spec.bits_per_sample) - 1);
            r.samples::<i32>()
                .map(|s| s.map(|v| v as f32 / scale))
                .collect::<Result<_, _>>()
        }
    }
    .map_err(|e| format!("{}: {e}", p.display()))?;
    let mut mono = vec![];
    downmix_into(&interleaved, usize::from(spec.channels), &mut mono).map_err(|e| e.to_string())?;
    if spec.sample_rate == 16_000 {
        return Ok(mono);
    }
    let mut rs = StreamResampler::new(spec.sample_rate).map_err(|e| e.to_string())?;
    let mut out = vec![];
    rs.push(&mono, &mut out).map_err(|e| e.to_string())?;
    rs.flush(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

// ---------- process metrics ----------

/// (current, peak) resident set in KiB. Peak is only available on Linux.
fn rss_kib(pid: u32) -> Option<(u64, Option<u64>)> {
    #[cfg(target_os = "linux")]
    {
        let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let field = |name: &str| {
            s.lines()
                .find(|l| l.starts_with(name))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        };
        Some((field("VmRSS:")?, field("VmHWM:")))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("/bin/ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let v = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some((v, None))
    }
}

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("/bin/kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn worker(a: &Args, epoch: u64) -> R<(WorkerClient, u64, Duration)> {
    let exe = std::fs::canonicalize(a.req("--worker")?).map_err(|e| e.to_string())?;
    let model = std::fs::canonicalize(a.req("--model")?).map_err(|e| e.to_string())?;
    let t0 = Instant::now();
    let mut c =
        WorkerClient::spawn(&exe, epoch, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    let load_ms = c
        .load_model(&model, a.req("--sha")?, Duration::from_secs(300))
        .map_err(|e| e.to_string())?;
    Ok((c, load_ms, t0.elapsed()))
}

fn job(epoch: u64) -> R<JobId> {
    Ok(JobId {
        session_id: SessionId::random().map_err(|e| e.to_string())?,
        generation: 0,
        worker_epoch: epoch,
    })
}

fn lang(s: &str) -> R<Language> {
    Language::try_from(s.to_string())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn host() -> serde_json::Value {
    json!({ "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "cpus": std::thread::available_parallelism().map_or(0, |n| n.get()) })
}

// ---------- corpus ----------

struct Utt {
    path: PathBuf,
    language: String,
    reference: String,
}

fn read_manifest(p: &Path) -> R<Vec<Utt>> {
    let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
    let base = p.parent().unwrap_or(Path::new("."));
    let mut v = vec![];
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut f = line.splitn(3, '\t');
        let (Some(path), Some(language), Some(reference)) = (f.next(), f.next(), f.next()) else {
            return Err(format!(
                "{}:{}: expected path<TAB>language<TAB>reference",
                p.display(),
                n + 1
            ));
        };
        v.push(Utt {
            path: base.join(path),
            language: language.into(),
            reference: reference.into(),
        });
    }
    Ok(v)
}

fn corpus(a: &Args) -> R<serde_json::Value> {
    let utts = read_manifest(Path::new(a.req("--manifest")?))?;
    let trials = a.num("--trials", 30)?.max(1);
    let cold = a.num("--cold", 10)?;
    let (mut c, load_ms, _) = worker(a, 1)?;
    let info = c.info().clone();
    let pid = c.pid().unwrap_or(0);

    let mut rows = vec![];
    let mut by_lang: std::collections::BTreeMap<String, (Counts, Counts, Counts, Vec<f64>)> =
        Default::default();
    let mut per_utt_wer = vec![];
    let mut peak_worker = 0u64;
    println!("# Corpus benchmark\n");
    println!(
        "engine {} {} · backends {:?} · model load {load_ms} ms · {} warm trials/utterance\n",
        info.engine, info.engine_revision, info.backends, trials
    );
    println!(
        "| file | lang | audio s | median ms | p95 ms | max ms | RTF | WER raw | WER norm | CER |"
    );
    println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|");
    for u in &utts {
        let pcm = load_wav(&u.path)?;
        let audio_s = pcm.len() as f64 / 16_000.0;
        let mut lat = vec![];
        let mut hyp = String::new();
        for _ in 0..trials {
            let t = Instant::now();
            let r = c
                .transcribe(job(1)?, lang(&u.language)?, &pcm, Duration::from_secs(600))
                .map_err(|e| e.to_string())?;
            lat.push(ms(t.elapsed()));
            hyp = r.text;
            if let Some((cur, peak)) = rss_kib(pid) {
                peak_worker = peak_worker.max(peak.unwrap_or(cur));
            }
        }
        let s = stats(&lat).ok_or("no trials")?;
        let (wr, wn, ce) = (
            wer_raw(&u.reference, &hyp),
            wer(&u.reference, &hyp),
            cer(&u.reference, &hyp),
        );
        per_utt_wer.push(wn.rate());
        let name = u
            .path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        println!(
            "| {name} | {} | {audio_s:.1} | {:.0} | {:.0} | {:.0} | {:.3} | {:.1}% | {:.1}% | {:.1}% |",
            u.language,
            s.median,
            s.p95,
            s.max,
            s.median / 1000.0 / audio_s,
            wr.rate() * 100.0,
            wn.rate() * 100.0,
            ce.rate() * 100.0
        );
        let e = by_lang.entry(u.language.clone()).or_default();
        e.0 = e.0.plus(wr);
        e.1 = e.1.plus(wn);
        e.2 = e.2.plus(ce);
        e.3.extend(&lat);
        rows.push(json!({ "file": name, "language": u.language, "audioSeconds": audio_s, "latencyMs": s, "werRaw": wr, "werNormalized": wn, "cer": ce, "reference": u.reference, "hypothesis": hyp }));
    }
    let _ = c.shutdown(Duration::from_secs(5));

    println!("\n| language | utterances | WER raw | WER norm (pooled) | CER |");
    println!("|---|---:|---:|---:|---:|");
    let mut langs = vec![];
    for (l, (wr, wn, ce, _)) in &by_lang {
        let n = rows.iter().filter(|r| r["language"] == l.as_str()).count();
        println!(
            "| {l} | {n} | {:.1}% | {:.1}% | {:.1}% |",
            wr.rate() * 100.0,
            wn.rate() * 100.0,
            ce.rate() * 100.0
        );
        langs.push(
            json!({ "language": l, "utterances": n, "werRaw": wr, "werNormalized": wn, "cer": ce }),
        );
    }
    let median_wer = stats(&per_utt_wer).map(|s| s.median);

    // Cold starts: fresh process + verified model load + first job.
    let mut cold_ms = vec![];
    let mut load_only = vec![];
    if let Some(first) = utts.first() {
        let pcm = load_wav(&first.path)?;
        for i in 0..cold {
            let epoch = 100 + i as u64;
            let t = Instant::now();
            let (mut c, load, _) = worker(a, epoch)?;
            c.transcribe(
                job(epoch)?,
                lang(&first.language)?,
                &pcm,
                Duration::from_secs(600),
            )
            .map_err(|e| e.to_string())?;
            cold_ms.push(ms(t.elapsed()));
            load_only.push(load as f64);
            let _ = c.shutdown(Duration::from_secs(5));
        }
    }
    let cold_s = stats(&cold_ms);
    let load_s = stats(&load_only);
    if let (Some(cs), Some(ls)) = (cold_s, load_s) {
        println!(
            "\ncold start (spawn + verified load + first job, {} runs): median {:.0} ms, p95 {:.0} ms, max {:.0} ms; model load alone median {:.0} ms",
            cs.n, cs.median, cs.p95, cs.max, ls.median
        );
    }
    println!(
        "worker peak RSS: {} MiB (includes model residency)",
        peak_worker / 1024
    );
    if let Some(m) = median_wer {
        println!("median per-utterance normalized WER: {:.1}%", m * 100.0);
    }
    Ok(json!({
        "kind": "corpus", "host": host(), "engine": info.engine, "engineRevision": info.engine_revision,
        "backends": info.backends, "modelSha256": a.req("--sha")?, "trials": trials,
        "warmModelLoadMs": load_ms, "utterances": rows, "languages": langs,
        "medianNormalizedWer": median_wer, "coldStartMs": cold_s, "coldModelLoadMs": load_s,
        "workerPeakRssKiB": peak_worker,
    }))
}

// ---------- no-speech ----------

fn run_pipeline(pcm: &[f32]) -> R<(EpOutcome, Vec<f64>)> {
    let mut p = Pipeline::new(
        16_000,
        1,
        Box::new(EarshotDetector::new(DetectorConfig::default())),
        EndpointConfig::default(),
    )
    .map_err(|e| e.to_string())?;
    let mut block_us = vec![];
    for chunk in pcm.chunks(160) {
        let t = Instant::now();
        let r = p.push(chunk).map_err(|e| e.to_string())?;
        block_us.push(t.elapsed().as_secs_f64() * 1e6);
        if let Some(o) = r {
            return Ok((o, block_us));
        }
    }
    Ok((p.stop().map_err(|e| e.to_string())?, block_us))
}

fn no_speech(a: &Args) -> R<serde_json::Value> {
    let seconds = a.num("--seconds", 8)?;
    let seeds = a.num("--seeds", 5)?;
    let mut cases: Vec<(String, Vec<f32>)> = vec![];
    for seed in 0..seeds {
        for (name, pcm) in nospeech::fixtures(seconds, seed as u64 + 1) {
            if seed > 0
                && (name == "silence" || name == "hum-50Hz" || name == "click" || name == "chord")
            {
                continue; // deterministic signals: one instance is enough
            }
            cases.push((format!("{name}#{seed}"), pcm));
        }
    }
    if let Some(dir) = a.get("--dir") {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "wav"))
            .collect();
        entries.sort();
        for p in entries {
            cases.push((
                p.file_name()
                    .map(|f| f.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                load_wav(&p)?,
            ));
        }
    }
    // With --worker/--model/--sha, accepted cases also go through inference:
    // only non-empty text would become an automated insertion (docs/08 §4).
    let mut w = match a.get("--worker") {
        Some(_) => Some(worker(a, 1)?.0),
        None => None,
    };
    let mut accepted = vec![];
    let mut inserted = vec![];
    let mut all_blocks = vec![];
    println!("# No-speech suite (pipeline: earshot VAD + endpointer, default settings)\n");
    println!("| case | endpointer | worker text |");
    println!("|---|---|---|");
    for (name, pcm) in &cases {
        let (o, blocks) = run_pipeline(pcm)?;
        all_blocks.extend(blocks);
        let mut text_col = String::from("–");
        let label = match &o {
            EpOutcome::Utterance {
                speech_ms, samples, ..
            } => {
                accepted.push(name.clone());
                if let Some(c) = w.as_mut() {
                    let r = c
                        .transcribe(job(1)?, lang("auto")?, samples, Duration::from_secs(600))
                        .map_err(|e| e.to_string())?;
                    let t = r.text.trim().to_owned();
                    text_col = if t.is_empty() {
                        "(empty → no usable text)".into()
                    } else {
                        inserted.push(json!({ "case": name, "text": t }));
                        format!("**{t:?}**")
                    };
                }
                format!("**ACCEPTED** ({speech_ms} ms speech)")
            }
            EpOutcome::NoSpeech => "no speech".into(),
            EpOutcome::TooShort => "too short".into(),
        };
        println!("| {name} | {label} | {text_col} |");
    }
    if let Some(c) = w.take() {
        let _ = c.shutdown(Duration::from_secs(5));
    }
    let n = cases.len();
    let k = accepted.len();
    let blocks = stats(&all_blocks);
    println!(
        "\nendpointer accepts: {k}/{n} (one-sided 95% upper bound {:.1}%)",
        upper_95(k, n) * 100.0
    );
    if a.get("--worker").is_some() {
        println!(
            "automated insertions (non-empty text): {}/{n} (one-sided 95% upper bound {:.1}%)",
            inserted.len(),
            upper_95(inserted.len(), n) * 100.0
        );
    }
    if let Some(b) = blocks {
        println!(
            "DSP time per 10 ms block: median {:.1} µs, p95 {:.1} µs, max {:.1} µs",
            b.median, b.p95, b.max
        );
    }
    Ok(
        json!({ "kind": "nospeech", "host": host(), "cases": n, "endpointerAccepts": accepted, "insertions": inserted, "upper95": upper_95(k, n), "blockMicros": blocks }),
    )
}

// ---------- soak ----------

fn soak(a: &Args) -> R<serde_json::Value> {
    let sessions = a.num("--sessions", 500)?;
    let kill_every = a.num("--kill-every", 10)?.max(2);
    let clip: Vec<f32> = {
        let p = a.get("--wav").map_or_else(
            || {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../vendor/whisper.cpp/samples/jfk.wav")
            },
            PathBuf::from,
        );
        load_wav(&p)?.into_iter().take(16_000 * 3).collect()
    };
    let mut epoch = 1u64;
    let (mut c, _, _) = worker(a, epoch)?;
    let mut samples: Vec<(usize, u64)> = vec![];
    let mut lat = vec![];
    let mut orphans = 0usize;
    let mut kills = 0usize;
    let started = Instant::now();
    for i in 0..sessions {
        if i > 0 && i % kill_every == 0 {
            // Cancel mid-job: hard kill from another thread, then respawn.
            let pid = c.pid().unwrap_or(0);
            let h = c.cancel_handle();
            let killer = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                h.kill();
            });
            let r = c.transcribe(job(epoch)?, lang("en")?, &clip, Duration::from_secs(120));
            let _ = killer.join();
            drop(r);
            c.kill();
            kills += 1;
            if pid != 0 && pid_alive(pid) {
                orphans += 1;
            }
            epoch += 1;
            c = worker(a, epoch)?.0;
            continue;
        }
        let t = Instant::now();
        c.transcribe(job(epoch)?, lang("en")?, &clip, Duration::from_secs(120))
            .map_err(|e| format!("session {i}: {e}"))?;
        lat.push(ms(t.elapsed()));
        if i % 25 == 1
            && let Some((cur, _)) = c.pid().and_then(rss_kib)
        {
            samples.push((i, cur));
        }
    }
    let _ = c.shutdown(Duration::from_secs(5));
    let parent = rss_kib(std::process::id()).map(|r| r.0);
    // Trend: compare the mean of the first and last thirds of samples after warm-up.
    let warm: Vec<u64> = samples.iter().skip(2).map(|s| s.1).collect();
    let third = (warm.len() / 3).max(1);
    let mean = |v: &[u64]| v.iter().sum::<u64>() as f64 / v.len().max(1) as f64;
    let (early, late) = (
        mean(&warm[..third.min(warm.len())]),
        mean(&warm[warm.len().saturating_sub(third)..]),
    );
    let growth = if early > 0.0 {
        (late - early) / early
    } else {
        0.0
    };
    let s = stats(&lat);
    println!("# Real-worker soak\n");
    println!(
        "{sessions} sessions in {:.0} s: {} jobs, {kills} hard kills + respawns, orphaned workers: {orphans}",
        started.elapsed().as_secs_f64(),
        lat.len()
    );
    if let Some(s) = s {
        println!(
            "3 s clip latency: median {:.0} ms, p95 {:.0} ms, max {:.0} ms",
            s.median, s.p95, s.max
        );
    }
    println!("worker RSS samples (session, KiB): {samples:?}");
    println!(
        "worker RSS trend after warm-up: {:+.1}% (investigate > +10%); parent RSS {parent:?} KiB",
        growth * 100.0
    );
    if orphans > 0 {
        return Err(format!("{orphans} orphaned worker processes"));
    }
    Ok(
        json!({ "kind": "soak", "host": host(), "sessions": sessions, "kills": kills, "orphans": orphans, "latencyMs": s, "workerRssKiB": samples, "rssGrowth": growth, "parentRssKiB": parent }),
    )
}
