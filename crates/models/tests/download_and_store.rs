//! Downloader and store against a scripted fake server (no network).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Mutex;

use dictation_models::download::{FetchResponse, Fetcher, download_to};
use dictation_models::{ApprovedModel, Cancel, ModelError, ModelState, ModelStore, Progress};
use sha2::{Digest, Sha256};

const URL: &str = "https://huggingface.co/org/repo/resolve/rev/test.bin";
const CDN: &str = "https://us.aws.cdn.hf.co/xet/abc?X-Amz-Signature=t";

fn data() -> Vec<u8> {
    (0..10_000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 24) as u8)
        .collect()
}

fn sha(b: &[u8]) -> &'static str {
    let h: String = Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect();
    Box::leak(h.into_boxed_str())
}

fn model() -> ApprovedModel {
    let d = data();
    ApprovedModel {
        id: "test-model",
        display_name: "Test",
        profile: "balanced",
        file_name: "test.bin",
        bytes: d.len() as u64,
        sha256: sha(&d),
        url: URL,
        engine_format: "ggml",
        multilingual: true,
        license: "test",
        source: "test",
    }
}

#[derive(Clone)]
enum Reply {
    Redirect(&'static str),
    /// 200 with `body`, optional content-length.
    Full(Vec<u8>, bool),
    /// 206 serving from the requested offset with this Content-Range override.
    Partial(Option<&'static str>),
    /// 200 ignoring Range.
    IgnoreRange,
    Status(u16),
}

struct Fake {
    routes: HashMap<String, Reply>,
    log: Mutex<Vec<(String, u64)>>,
}

impl Fake {
    fn new(routes: &[(&str, Reply)]) -> Self {
        Self {
            routes: routes
                .iter()
                .map(|(u, r)| (u.to_string(), r.clone()))
                .collect(),
            log: Mutex::new(vec![]),
        }
    }
    fn requests(&self) -> Vec<(String, u64)> {
        self.log.lock().unwrap().clone()
    }
}

impl Fetcher for Fake {
    fn get(&self, url: &str, range_start: u64) -> Result<FetchResponse, String> {
        self.log
            .lock()
            .unwrap()
            .push((url.to_string(), range_start));
        let d = data();
        let resp = |status, body: Vec<u8>, cr: Option<String>, len: Option<u64>| FetchResponse {
            status,
            location: None,
            content_range: cr,
            content_length: len,
            body: Box::new(Cursor::new(body)),
        };
        Ok(match self.routes.get(url).cloned().ok_or("no route")? {
            Reply::Redirect(to) => FetchResponse {
                location: Some(to.into()),
                ..resp(302, vec![], None, None)
            },
            Reply::Full(body, with_len) => {
                let len = with_len.then_some(body.len() as u64);
                resp(200, body, None, len)
            }
            Reply::IgnoreRange => resp(200, d.clone(), None, Some(d.len() as u64)),
            Reply::Partial(cr) => {
                let start = range_start as usize;
                let cr = cr
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("bytes {start}-{}/{}", d.len() - 1, d.len()));
                resp(
                    206,
                    d[start..].to_vec(),
                    Some(cr),
                    Some((d.len() - start) as u64),
                )
            }
            Reply::Status(s) => resp(s, vec![], None, None),
        })
    }
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dictation-models-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(
    fake: &Fake,
    partial: &PathBuf,
    cancel_at: Option<u64>,
) -> Result<Vec<Progress>, ModelError> {
    let cancel = Cancel::new();
    let mut seen = vec![];
    let c = cancel.clone();
    download_to(
        &model(),
        partial,
        fake,
        &mut |p| {
            if cancel_at.is_some_and(|n| p.done >= n) {
                c.set();
            }
            seen.push(p);
        },
        &cancel,
    )
    .map(|()| seen)
}

#[test]
fn redirect_to_cdn_then_verified_body() {
    let dir = tmp("ok");
    let partial = dir.join("test.bin.partial");
    let fake = Fake::new(&[
        (URL, Reply::Redirect(CDN)),
        (CDN, Reply::Full(data(), true)),
    ]);
    let progress = run(&fake, &partial, None).unwrap();
    assert_eq!(std::fs::read(&partial).unwrap(), data());
    assert_eq!(progress.last().unwrap().done, data().len() as u64);
    assert!(
        progress.windows(2).all(|w| w[0].done <= w[1].done),
        "progress must be monotone"
    );
}

#[test]
fn disallowed_redirects_are_refused() {
    for (to, err) in [
        ("http://us.aws.cdn.hf.co/x", "DisallowedUrl"),
        ("https://evil.example.com/x", "DisallowedUrl"),
        ("https://huggingface.co.evil.com/x", "DisallowedUrl"),
    ] {
        let dir = tmp("redir");
        let fake = Fake::new(&[(
            URL,
            Reply::Redirect(Box::leak(to.to_string().into_boxed_str())),
        )]);
        let e = run(&fake, &dir.join("p"), None).unwrap_err();
        assert!(format!("{e:?}").starts_with(err), "{to}: {e:?}");
        assert_eq!(fake.requests().len(), 1, "never contacted {to}");
    }
    // Redirect loop.
    let fake = Fake::new(&[(URL, Reply::Redirect(URL))]);
    assert_eq!(
        run(&fake, &tmp("loop").join("p"), None).unwrap_err(),
        ModelError::TooManyRedirects
    );
    assert_eq!(fake.requests().len(), 6);
}

#[test]
fn oversized_and_wrong_length_bodies_are_rejected_and_deleted() {
    let mut big = data();
    big.extend([0u8; 500]);
    for with_len in [false, true] {
        let dir = tmp("big");
        let partial = dir.join("p");
        let fake = Fake::new(&[(URL, Reply::Full(big.clone(), with_len))]);
        assert_eq!(
            run(&fake, &partial, None).unwrap_err(),
            ModelError::TooLarge
        );
        assert!(!partial.exists(), "oversized partial must be deleted");
    }
}

#[test]
fn hash_mismatch_deletes_partial() {
    let dir = tmp("hash");
    let partial = dir.join("p");
    let mut wrong = data();
    wrong[123] ^= 0xff;
    let fake = Fake::new(&[(URL, Reply::Full(wrong, true))]);
    assert_eq!(
        run(&fake, &partial, None).unwrap_err(),
        ModelError::HashMismatch
    );
    assert!(!partial.exists());
}

#[test]
fn short_body_is_incomplete_and_resumable() {
    let dir = tmp("short");
    let partial = dir.join("p");
    let fake = Fake::new(&[(URL, Reply::Full(data()[..4000].to_vec(), false))]);
    assert_eq!(
        run(&fake, &partial, None).unwrap_err(),
        ModelError::Incomplete {
            got: 4000,
            expected: 10_000
        }
    );
    assert_eq!(std::fs::metadata(&partial).unwrap().len(), 4000);
    // Resume with a correct 206.
    let fake = Fake::new(&[(URL, Reply::Partial(None))]);
    run(&fake, &partial, None).unwrap();
    assert_eq!(fake.requests(), vec![(URL.to_string(), 4000)]);
    assert_eq!(std::fs::read(&partial).unwrap(), data());
}

#[test]
fn cancel_keeps_partial_then_resume_completes() {
    let dir = tmp("cancel");
    let partial = dir.join("p");
    // Body arrives in one 10_000-byte read; cancel triggers at the next chunk check.
    let fake = Fake::new(&[(URL, Reply::Full(data(), true))]);
    assert_eq!(
        run(&fake, &partial, Some(1)).unwrap_err(),
        ModelError::Canceled
    );
    let kept = std::fs::metadata(&partial).unwrap().len();
    assert!(kept > 0);
    let fake = Fake::new(&[(URL, Reply::Partial(None))]);
    if kept < 10_000 {
        run(&fake, &partial, None).unwrap();
    } else {
        // Whole body already written: completes with no request.
        run(&fake, &partial, None).unwrap();
        assert!(fake.requests().is_empty());
    }
    assert_eq!(std::fs::read(&partial).unwrap(), data());
}

#[test]
fn bad_resume_responses_restart_from_zero() {
    for reply in [
        Reply::IgnoreRange,
        Reply::Partial(Some("bytes 0-9999/10000")),
    ] {
        let dir = tmp("restart");
        let partial = dir.join("p");
        std::fs::write(&partial, &data()[..3000]).unwrap();
        let fake = Fake::new(&[(URL, reply.clone())]);
        // The first answer cannot be trusted as a resume (full body, or a
        // Content-Range that does not start at 3000), so the downloader
        // discards the partial and restarts from 0, where the response is valid.
        run(&fake, &partial, None).unwrap();
        assert_eq!(std::fs::read(&partial).unwrap(), data());
        let offsets: Vec<u64> = fake.requests().iter().map(|r| r.1).collect();
        assert_eq!(offsets, vec![3000, 0]);
        assert_eq!(fake.requests()[0].1, 3000, "first attempt resumes");
    }
}

#[test]
fn http_errors_surface_without_deleting_partial() {
    let dir = tmp("err");
    let partial = dir.join("p");
    std::fs::write(&partial, &data()[..100]).unwrap();
    let fake = Fake::new(&[(URL, Reply::Status(503))]);
    assert_eq!(
        run(&fake, &partial, None).unwrap_err(),
        ModelError::Network("HTTP 503".into())
    );
    assert_eq!(std::fs::metadata(&partial).unwrap().len(), 100);
}

#[test]
fn store_install_import_verify_remove() {
    let dir = tmp("store");
    let store = ModelStore::open(&dir.join("models")).unwrap();
    let m = model();
    assert_eq!(store.state(&m), ModelState::NotInstalled);

    // Partial download never counts as installed.
    std::fs::write(dir.join("models/test.bin.partial"), b"abc").unwrap();
    assert_eq!(store.state(&m), ModelState::Partial { bytes: 3 });
    assert!(store.installed_path(&m).is_none());

    let fake = Fake::new(&[(URL, Reply::IgnoreRange)]);
    store
        .install(&m, &fake, &mut |_| {}, &Cancel::new())
        .unwrap();
    assert_eq!(store.state(&m), ModelState::Installed { verified: true });
    assert!(!dir.join("models/test.bin.partial").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.join("models"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(store.path(&m))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    // Corrupting the installed file is caught by verify, and the marker no longer matches.
    let p = store.path(&m);
    let mut bytes = std::fs::read(&p).unwrap();
    bytes[5000] ^= 1;
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&p, &bytes).unwrap();
    assert_eq!(store.state(&m), ModelState::Installed { verified: false });
    assert_eq!(
        store.verify(&m, &Cancel::new()),
        Err(ModelError::HashMismatch)
    );

    store.remove(&m).unwrap();
    assert_eq!(store.state(&m), ModelState::NotInstalled);

    // Import: wrong size, wrong content, then the right file.
    let src = dir.join("src.bin");
    std::fs::write(&src, &data()[..9000]).unwrap();
    assert_eq!(
        store.import(&m, &src, &mut |_| {}, &Cancel::new()),
        Err(ModelError::WrongSize {
            got: 9000,
            expected: 10_000
        })
    );
    std::fs::write(&src, &bytes).unwrap();
    assert_eq!(
        store.import(&m, &src, &mut |_| {}, &Cancel::new()),
        Err(ModelError::HashMismatch)
    );
    assert_eq!(
        store.state(&m),
        ModelState::NotInstalled,
        "failed import leaves nothing behind"
    );
    std::fs::write(&src, data()).unwrap();
    store.import(&m, &src, &mut |_| {}, &Cancel::new()).unwrap();
    assert_eq!(store.state(&m), ModelState::Installed { verified: true });
    assert!(src.exists(), "import copies; the source is untouched");
    assert_eq!(
        store.import(&m, &dir, &mut |_| {}, &Cancel::new()),
        Err(ModelError::NotAFile)
    );
}

#[cfg(unix)]
#[test]
fn symlinked_model_file_is_invalid() {
    let dir = tmp("symlink");
    let store = ModelStore::open(&dir.join("models")).unwrap();
    let m = model();
    let real = dir.join("real.bin");
    std::fs::write(&real, data()).unwrap();
    std::os::unix::fs::symlink(&real, store.path(&m)).unwrap();
    assert_eq!(store.state(&m), ModelState::Invalid);
    assert!(store.installed_path(&m).is_none());
}
