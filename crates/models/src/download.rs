//! Verified, resumable download into a `.partial` file.
//!
//! Rules (docs/06 §5, docs/03 §6): HTTPS only; every hop (including
//! redirects) must be an approved host; at most `MAX_REDIRECTS`; the body may
//! not exceed the manifest size; resume only when the server answers 206 with
//! the exact expected `Content-Range`, otherwise restart from zero; SHA-256 is
//! computed over the final bytes and compared with the bundled manifest.
//! Requests carry no transcript, device or target identifiers.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::manifest::{ApprovedModel, MAX_REDIRECTS, host_allowed};
use crate::store::{Cancel, ModelError, Progress};

const CHUNK: usize = 1 << 20;

pub struct FetchResponse {
    pub status: u16,
    pub location: Option<String>,
    pub content_range: Option<String>,
    pub content_length: Option<u64>,
    pub body: Box<dyn Read + Send>,
}

/// One HTTP GET without following redirects. `range_start > 0` asks for
/// `Range: bytes=<start>-`.
pub trait Fetcher: Send + Sync {
    fn get(&self, url: &str, range_start: u64) -> Result<FetchResponse, String>;
}

/// Returns the lowercase host of an `https://` URL, rejecting userinfo and
/// non-default ports.
pub fn https_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let host = match authority.rsplit_once(':') {
        Some((h, "443")) => h,
        Some(_) => return None,
        None => authority,
    };
    Some(host.to_ascii_lowercase())
}

fn check_url(url: &str) -> Result<(), ModelError> {
    match https_host(url) {
        Some(h) if host_allowed(&h) => Ok(()),
        _ => Err(ModelError::DisallowedUrl(redact(url))),
    }
}

/// Keep scheme and host only: CDN URLs carry signed query tokens.
fn redact(url: &str) -> String {
    match url.find("://") {
        Some(i) => {
            let after = &url[i + 3..];
            let host = after.split(['/', '?', '#']).next().unwrap_or("");
            format!("{}://{host}/…", &url[..i])
        }
        None => "<invalid url>".into(),
    }
}

fn parse_content_range(v: &str) -> Option<(u64, u64, u64)> {
    let r = v.trim().strip_prefix("bytes ")?;
    let (span, total) = r.split_once('/')?;
    let (a, b) = span.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()?, total.parse().ok()?))
}

/// Downloads `model` into `partial`, resuming if possible. On success the
/// partial file holds exactly the verified bytes (fsynced); the caller
/// renames it. On cancel or a transient error the partial is kept for resume;
/// on a hash mismatch or oversized body it is deleted.
pub fn download_to(
    model: &ApprovedModel,
    partial: &Path,
    fetcher: &dyn Fetcher,
    progress: &mut dyn FnMut(Progress),
    cancel: &Cancel,
) -> Result<(), ModelError> {
    check_url(model.url)?;
    let mut file = open_private_rw(partial)?;
    let mut have = file.metadata()?.len();
    if have > model.bytes {
        file.set_len(0)?;
        have = 0;
    }
    let mut restarted = false;

    loop {
        // Re-seed the hash with the bytes we already have.
        let mut hasher = Sha256::new();
        if have > 0 {
            file.seek(SeekFrom::Start(0))?;
            hash_prefix(&mut file, have, &mut hasher, cancel)?;
        }
        if have == model.bytes {
            return finish(model, partial, file, hasher);
        }

        let resp = follow(model.url, have, fetcher)?;
        match (resp.status, have) {
            (200, 0) => {}
            (200, _) => {
                // Server ignored the range: restart from scratch, once.
                if restarted {
                    return Err(ModelError::BadRange);
                }
                restarted = true;
                file.set_len(0)?;
                have = 0;
                continue;
            }
            (206, _) => {
                let ok = resp
                    .content_range
                    .as_deref()
                    .and_then(parse_content_range)
                    .is_some_and(|(a, b, t)| a == have && t == model.bytes && b == model.bytes - 1);
                if !ok {
                    if restarted {
                        return Err(ModelError::BadRange);
                    }
                    restarted = true;
                    file.set_len(0)?;
                    have = 0;
                    continue;
                }
            }
            (s, _) => return Err(ModelError::Network(format!("HTTP {s}"))),
        }
        if let Some(len) = resp.content_length
            && have + len != model.bytes
        {
            drop(file);
            let _ = std::fs::remove_file(partial);
            return Err(ModelError::TooLarge);
        }

        file.seek(SeekFrom::Start(have))?;
        let mut body = resp.body;
        let mut buf = vec![0u8; CHUNK];
        let mut done = have;
        progress(Progress {
            id: model.id,
            done,
            total: model.bytes,
        });
        loop {
            if cancel.is_set() {
                file.sync_all()?;
                return Err(ModelError::Canceled);
            }
            let n = match body.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    file.sync_all()?;
                    return Err(ModelError::Network(e.to_string()));
                }
            };
            if done + n as u64 > model.bytes {
                drop(file);
                let _ = std::fs::remove_file(partial);
                return Err(ModelError::TooLarge);
            }
            file.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            done += n as u64;
            progress(Progress {
                id: model.id,
                done,
                total: model.bytes,
            });
        }
        if done != model.bytes {
            file.sync_all()?;
            return Err(ModelError::Incomplete {
                got: done,
                expected: model.bytes,
            });
        }
        return finish(model, partial, file, hasher);
    }
}

fn follow(start: &str, range: u64, fetcher: &dyn Fetcher) -> Result<FetchResponse, ModelError> {
    let mut url = start.to_owned();
    for _ in 0..=MAX_REDIRECTS {
        check_url(&url)?;
        let resp = fetcher.get(&url, range).map_err(ModelError::Network)?;
        if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
            let next = resp
                .location
                .ok_or_else(|| ModelError::Network("redirect without location".into()))?;
            if !next.starts_with("https://") {
                return Err(ModelError::DisallowedUrl(redact(&next)));
            }
            url = next;
            continue;
        }
        return Ok(resp);
    }
    Err(ModelError::TooManyRedirects)
}

fn hash_prefix(
    file: &mut File,
    len: u64,
    hasher: &mut Sha256,
    cancel: &Cancel,
) -> Result<(), ModelError> {
    let mut buf = vec![0u8; CHUNK];
    let mut left = len;
    while left > 0 {
        if cancel.is_set() {
            return Err(ModelError::Canceled);
        }
        let n = (left as usize).min(CHUNK);
        file.read_exact(&mut buf[..n])?;
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    Ok(())
}

fn finish(
    model: &ApprovedModel,
    partial: &Path,
    file: File,
    hasher: Sha256,
) -> Result<(), ModelError> {
    let got = hex(&hasher.finalize());
    if got != model.sha256 {
        drop(file);
        let _ = std::fs::remove_file(partial);
        return Err(ModelError::HashMismatch);
    }
    file.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn open_private_rw(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    // `mode` applies only on creation; tighten a pre-existing partial too.
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    Ok(f)
}

#[cfg(not(unix))]
fn open_private_rw(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Real HTTPS fetcher (ureq + rustls). Redirects are never followed
/// automatically so every hop passes the host allowlist.
#[cfg(feature = "http")]
pub struct HttpFetcher {
    agent: ureq::Agent,
}

#[cfg(feature = "http")]
impl HttpFetcher {
    pub fn new(user_agent: &str) -> Self {
        use std::time::Duration;
        let config = ureq::Agent::config_builder()
            .https_only(true)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .user_agent(user_agent)
            .build();
        Self {
            agent: config.into(),
        }
    }
}

#[cfg(feature = "http")]
impl Fetcher for HttpFetcher {
    fn get(&self, url: &str, range_start: u64) -> Result<FetchResponse, String> {
        let mut req = self.agent.get(url);
        if range_start > 0 {
            req = req.header("Range", format!("bytes={range_start}-"));
        }
        let resp = req.call().map_err(|e| e.to_string())?;
        let header = |name: &str| {
            resp.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
        };
        let status = resp.status().as_u16();
        let location = header("location");
        let content_range = header("content-range");
        let content_length = header("content-length").and_then(|v| v.parse().ok());
        let body = resp.into_body().into_with_config().limit(u64::MAX).reader();
        Ok(FetchResponse {
            status,
            location,
            content_range,
            content_length,
            body: Box::new(body),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_host_parsing() {
        assert_eq!(
            https_host("https://huggingface.co/x").as_deref(),
            Some("huggingface.co")
        );
        assert_eq!(
            https_host("https://US.aws.cdn.hf.co:443/a?b").as_deref(),
            Some("us.aws.cdn.hf.co")
        );
        for bad in [
            "http://huggingface.co/x",
            "https://user@huggingface.co/",
            "https://huggingface.co:8443/",
            "https:///x",
            "ftp://x",
        ] {
            assert_eq!(https_host(bad), None, "{bad}");
        }
    }

    #[test]
    fn content_range_parsing() {
        assert_eq!(
            parse_content_range("bytes 100-199/200"),
            Some((100, 199, 200))
        );
        assert_eq!(parse_content_range("bytes */200"), None);
    }

    #[test]
    fn redaction_drops_paths_and_tokens() {
        assert_eq!(
            redact("https://cdn.hf.co/a/b?X-Amz-Signature=secret"),
            "https://cdn.hf.co/…"
        );
    }
}
