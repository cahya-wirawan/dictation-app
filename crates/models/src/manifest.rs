//! Approved models. Values are pinned at build time and were verified
//! independently: `ggml-base.bin` by download and SHA-256 in M0, and
//! `ggml-small.bin` by a separate full download on 2026-11-12, both matching
//! the Hugging Face LFS metadata. Checksums fetched at runtime from the
//! download server are never trusted on their own (docs/06 §5).

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovedModel {
    pub id: &'static str,
    pub display_name: &'static str,
    /// "balanced" (default candidate) or "quality".
    pub profile: &'static str,
    pub file_name: &'static str,
    pub bytes: u64,
    #[serde(skip)]
    pub sha256: &'static str,
    #[serde(skip)]
    pub url: &'static str,
    /// whisper.cpp ggml format; works with the pinned engine (v1.9.5).
    pub engine_format: &'static str,
    pub multilingual: bool,
    pub license: &'static str,
    pub source: &'static str,
}

/// Immutable repository revision; `main` could change under a pinned hash.
pub const PINNED_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

pub const APPROVED: &[ApprovedModel] = &[
    ApprovedModel {
        id: "whisper-base-multilingual",
        display_name: "Whisper Base (multilingual)",
        profile: "balanced",
        file_name: "ggml-base.bin",
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
        url: const_format_url::BASE,
        engine_format: "ggml (whisper.cpp)",
        multilingual: true,
        license: "MIT (OpenAI Whisper weights; ggml conversion by whisper.cpp)",
        source: "huggingface.co/ggerganov/whisper.cpp",
    },
    ApprovedModel {
        id: "whisper-small-multilingual",
        display_name: "Whisper Small (multilingual)",
        profile: "quality",
        file_name: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
        url: const_format_url::SMALL,
        engine_format: "ggml (whisper.cpp)",
        multilingual: true,
        license: "MIT (OpenAI Whisper weights; ggml conversion by whisper.cpp)",
        source: "huggingface.co/ggerganov/whisper.cpp",
    },
];

mod const_format_url {
    // Kept as literals so the pinned revision is visible in one place above.
    pub const BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.bin";
    pub const SMALL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.bin";
}

/// Hosts a download may start at or be redirected to (HTTPS only).
pub const ALLOWED_HOSTS: &[&str] = &["huggingface.co"];
/// Host suffixes for the Hugging Face CDN redirect targets (e.g. us.aws.cdn.hf.co).
pub const ALLOWED_HOST_SUFFIXES: &[&str] = &[".hf.co"];
pub const MAX_REDIRECTS: u32 = 5;

pub fn find(id: &str) -> Option<&'static ApprovedModel> {
    APPROVED.iter().find(|m| m.id == id)
}

pub fn host_allowed(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    ALLOWED_HOSTS.contains(&host.as_str())
        || ALLOWED_HOST_SUFFIXES
            .iter()
            .any(|s| host.ends_with(s) && host.len() > s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_is_well_formed_and_pinned() {
        for m in APPROVED {
            assert_eq!(m.sha256.len(), 64);
            assert!(
                m.sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            );
            assert!(m.url.starts_with("https://huggingface.co/"));
            assert!(
                m.url.contains(PINNED_REVISION),
                "{} must pin the repo revision",
                m.id
            );
            assert!(m.url.ends_with(m.file_name));
            assert!(!m.file_name.contains('/'));
        }
        assert_eq!(
            find("whisper-base-multilingual").map(|m| m.bytes),
            Some(147_951_465)
        );
        assert!(find("../etc/passwd").is_none());
    }

    #[test]
    fn host_allowlist() {
        for ok in [
            "huggingface.co",
            "us.aws.cdn.hf.co",
            "cas-bridge.xethub.hf.co",
            "HUGGINGFACE.CO",
        ] {
            assert!(host_allowed(ok), "{ok}");
        }
        for bad in [
            "hf.co.evil.com",
            "evilhuggingface.co",
            "huggingface.co.evil",
            ".hf.co",
            "example.com",
            "hf.co",
        ] {
            assert!(!host_allowed(bad), "{bad}");
        }
    }
}
