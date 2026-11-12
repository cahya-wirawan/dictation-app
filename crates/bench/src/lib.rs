//! Benchmark building blocks (docs/08 §3–§6). Pure and unit-tested; the CLI
//! in `main.rs` drives the real worker.
//!
//! Transcript normalization policy, fixed before any evaluation (docs/08 §3):
//!
//! * **raw**: Unicode NFC, whitespace-separated tokens, nothing else.
//! * **normalized**: NFC → Unicode lowercase → apostrophes removed inside
//!   words (`don't` → `dont`) → every other character that is not a letter,
//!   digit or whitespace becomes a space → whitespace collapsed.
//! * Numbers are **not** normalized: `10` and `ten` are different tokens.
//!   Diacritics are kept (`für` ≠ `fur`), so CER shows diacritic errors.

use unicode_normalization::UnicodeNormalization;

pub fn normalize_raw(s: &str) -> Vec<String> {
    s.nfc()
        .collect::<String>()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

pub fn normalize(s: &str) -> Vec<String> {
    let lowered: String = s.nfc().collect::<String>().to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    for c in lowered.chars() {
        if c == '\'' || c == '\u{2019}' {
            continue;
        }
        if c.is_alphanumeric() || c.is_whitespace() {
            out.push(c);
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().map(str::to_owned).collect()
}

/// Levenshtein distance over arbitrary tokens (substitution, insertion, deletion = 1).
pub fn edit_distance<T: PartialEq>(a: &[T], b: &[T]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, x) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(x != y);
            cur[j + 1] = sub.min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Errors and reference length, so rates can be pooled across utterances.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Counts {
    pub errors: usize,
    pub reference: usize,
}

impl Counts {
    pub fn rate(self) -> f64 {
        if self.reference == 0 {
            if self.errors == 0 { 0.0 } else { 1.0 }
        } else {
            self.errors as f64 / self.reference as f64
        }
    }
    pub fn plus(self, o: Counts) -> Counts {
        Counts {
            errors: self.errors + o.errors,
            reference: self.reference + o.reference,
        }
    }
}

pub fn wer_raw(reference: &str, hypothesis: &str) -> Counts {
    let (r, h) = (normalize_raw(reference), normalize_raw(hypothesis));
    Counts {
        errors: edit_distance(&r, &h),
        reference: r.len(),
    }
}

pub fn wer(reference: &str, hypothesis: &str) -> Counts {
    let (r, h) = (normalize(reference), normalize(hypothesis));
    Counts {
        errors: edit_distance(&r, &h),
        reference: r.len(),
    }
}

/// Character error rate on the normalized text (single spaces kept).
pub fn cer(reference: &str, hypothesis: &str) -> Counts {
    let r: Vec<char> = normalize(reference).join(" ").chars().collect();
    let h: Vec<char> = normalize(hypothesis).join(" ").chars().collect();
    Counts {
        errors: edit_distance(&r, &h),
        reference: r.len(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Stats {
    pub n: usize,
    pub median: f64,
    pub p95: f64,
    pub max: f64,
}

/// Nearest-rank median/p95/max; `None` for an empty sample.
pub fn stats(values: &[f64]) -> Option<Stats> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let rank = |p: f64| v[((p / 100.0 * v.len() as f64).ceil() as usize).clamp(1, v.len()) - 1];
    Some(Stats {
        n: v.len(),
        median: rank(50.0),
        p95: rank(95.0),
        max: v[v.len() - 1],
    })
}

/// One-sided 95% upper bound on a rate when `k` of `n` trials failed.
/// For k = 0 this is the "rule of three" (≈ 3/n); otherwise a Wilson bound.
pub fn upper_95(k: usize, n: usize) -> f64 {
    if n == 0 {
        return 1.0;
    }
    let nf = n as f64;
    if k == 0 {
        return 1.0 - 0.05f64.powf(1.0 / nf);
    }
    let z = 1.645;
    let p = k as f64 / nf;
    let denom = 1.0 + z * z / nf;
    let centre = p + z * z / (2.0 * nf);
    let margin = z * (p * (1.0 - p) / nf + z * z / (4.0 * nf * nf)).sqrt();
    ((centre + margin) / denom).min(1.0)
}

/// Deterministic synthetic no-speech fixtures at 16 kHz mono (docs/08 §3:
/// silence, fan-like noise, typing-like clicks, hum, music-like tones). They
/// complement, not replace, recorded no-speech material.
pub mod nospeech {
    use std::f32::consts::PI;

    pub const RATE: usize = 16_000;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        }
    }

    pub fn fixtures(seconds: usize, seed: u64) -> Vec<(String, Vec<f32>)> {
        let n = seconds * RATE;
        let mut rng = Lcg(seed);
        let mut out = vec![("silence".to_string(), vec![0.0; n])];
        for (name, amp) in [
            ("hiss-55dB", 0.0018f32),
            ("white-35dB", 0.018),
            ("white-25dB", 0.056),
        ] {
            out.push((name.into(), (0..n).map(|_| rng.next() * amp).collect()));
        }
        // Fan: low-passed (one-pole) noise, mostly < 500 Hz.
        let mut y = 0.0f32;
        out.push((
            "fan".into(),
            (0..n)
                .map(|_| {
                    y += 0.08 * (rng.next() * 0.25 - y);
                    y
                })
                .collect(),
        ));
        // Mains hum with harmonics.
        out.push((
            "hum-50Hz".into(),
            (0..n)
                .map(|i| {
                    let t = i as f32 / RATE as f32;
                    0.05 * (2.0 * PI * 50.0 * t).sin() + 0.02 * (2.0 * PI * 150.0 * t).sin()
                })
                .collect(),
        ));
        // Typing: short decaying noise bursts at irregular ~5–9 Hz intervals.
        let mut typing = vec![0.0f32; n];
        let mut pos = 0usize;
        while pos < n {
            for k in 0..240 {
                if let Some(s) = typing.get_mut(pos + k) {
                    *s = rng.next() * 0.3 * (-(k as f32) / 40.0).exp();
                }
            }
            pos += 1800 + ((rng.next().abs() * 1400.0) as usize);
        }
        out.push(("typing".into(), typing));
        // Single click.
        let mut click = vec![0.0f32; n];
        for (k, s) in click.iter_mut().skip(RATE).take(80).enumerate() {
            *s = 0.8 * (-(k as f32) / 10.0).exp();
        }
        out.push(("click".into(), click));
        // Music-like: a sustained chord with slow tremolo.
        out.push((
            "chord".into(),
            (0..n)
                .map(|i| {
                    let t = i as f32 / RATE as f32;
                    let env = 0.6 + 0.4 * (2.0 * PI * 0.5 * t).sin();
                    env * 0.05
                        * ((2.0 * PI * 261.6 * t).sin()
                            + (2.0 * PI * 329.6 * t).sin()
                            + (2.0 * PI * 392.0 * t).sin())
                })
                .collect(),
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn normalization_policy() {
        assert_eq!(
            normalize("Hello, World!  Don't"),
            ["hello", "world", "dont"]
        );
        // NFC: decomposed ü equals composed ü.
        assert_eq!(normalize("fu\u{0308}r"), normalize("für"));
        assert_eq!(normalize("für"), ["für"]);
        assert_eq!(normalize_raw("A  b."), ["A", "b."]);
        assert_eq!(normalize("10 ten"), ["10", "ten"]);
    }

    #[test]
    fn wer_and_cer() {
        assert_eq!(wer("the cat sat", "the cat sat").errors, 0);
        assert_eq!(
            wer("the cat sat", "the bat sat"),
            Counts {
                errors: 1,
                reference: 3
            }
        );
        assert_eq!(wer("a b c d", "a c d e").errors, 2);
        assert_eq!(
            wer("And so, my fellow Americans", "and so my fellow americans").errors,
            0
        );
        assert_eq!(wer_raw("And so, my", "and so my").errors, 2);
        assert_eq!(
            cer("für", "fur"),
            Counts {
                errors: 1,
                reference: 3
            }
        );
        assert!((wer("", "x").rate() - 1.0).abs() < 1e-9);
        assert!(wer("", "").rate().abs() < 1e-9);
    }

    #[test]
    fn stats_and_bounds() {
        let s = stats(&(1..=100).map(f64::from).collect::<Vec<_>>()).unwrap();
        assert_eq!((s.median, s.p95, s.max), (50.0, 95.0, 100.0));
        assert!(stats(&[]).is_none());
        // Rule of three: 0 of 60 → about 5%.
        let u = upper_95(0, 60);
        assert!((0.045..0.055).contains(&u), "{u}");
        assert!(upper_95(5, 100) > 0.05);
    }

    #[test]
    fn fixtures_are_deterministic_and_bounded() {
        let a = nospeech::fixtures(2, 7);
        let b = nospeech::fixtures(2, 7);
        assert_eq!(a.len(), b.len());
        for ((na, xa), (_, xb)) in a.iter().zip(&b) {
            assert_eq!(xa, xb, "{na}");
            assert_eq!(xa.len(), 2 * nospeech::RATE);
            assert!(xa.iter().all(|s| s.abs() <= 1.0), "{na}");
        }
    }
}
