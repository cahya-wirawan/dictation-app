//! Private model directory: inventory, install, import, verify and remove.
//!
//! Layout: `<dir>/<file_name>` is an installed model; `<file_name>.partial`
//! is an in-progress download (never counted as installed);
//! `<file_name>.verified` records the size, mtime and hash of the last full
//! verification so startup does not re-hash every model.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::download::{Fetcher, download_to, hex};
use crate::manifest::{APPROVED, ApprovedModel};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("unknown model id")]
    UnknownModel,
    #[error("network error: {0}")]
    Network(String),
    #[error("download URL not allowed: {0}")]
    DisallowedUrl(String),
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("server sent an invalid range response")]
    BadRange,
    #[error("download is larger than the approved size")]
    TooLarge,
    #[error("download incomplete ({got} of {expected} bytes); it can be resumed")]
    Incomplete { got: u64, expected: u64 },
    #[error("file does not match the approved checksum")]
    HashMismatch,
    #[error("file is {got} bytes; the approved model is {expected} bytes")]
    WrongSize { got: u64, expected: u64 },
    #[error("selected path is not a regular file")]
    NotAFile,
    #[error("canceled")]
    Canceled,
    #[error("file error: {0}")]
    Io(String),
}

impl From<std::io::Error> for ModelError {
    fn from(e: std::io::Error) -> Self {
        ModelError::Io(e.kind().to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub id: &'static str,
    pub done: u64,
    pub total: u64,
}

/// Cooperative cancel flag for one operation.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn set(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_set(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ModelState {
    NotInstalled,
    /// Resumable download present.
    Partial {
        bytes: u64,
    },
    /// Right size; `verified` is true if the full hash was checked for this
    /// exact file (size + mtime).
    Installed {
        verified: bool,
    },
    /// Present but the wrong size or a failed verification: never loaded.
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub model: ApprovedModel,
    pub state: ModelState,
}

pub struct ModelStore {
    dir: PathBuf,
}

impl ModelStore {
    pub fn open(dir: &Path) -> Result<Self, ModelError> {
        create_private_dir(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self, m: &ApprovedModel) -> PathBuf {
        self.dir.join(m.file_name)
    }

    fn partial(&self, m: &ApprovedModel) -> PathBuf {
        self.dir.join(format!("{}.partial", m.file_name))
    }

    fn marker(&self, m: &ApprovedModel) -> PathBuf {
        self.dir.join(format!("{}.verified", m.file_name))
    }

    pub fn inventory(&self) -> Vec<ModelStatus> {
        APPROVED
            .iter()
            .map(|m| ModelStatus {
                model: *m,
                state: self.state(m),
            })
            .collect()
    }

    pub fn state(&self, m: &ApprovedModel) -> ModelState {
        match fs::symlink_metadata(self.path(m)) {
            Ok(meta) if meta.file_type().is_file() && meta.len() == m.bytes => {
                ModelState::Installed {
                    verified: self.marker_matches(m, &meta),
                }
            }
            Ok(_) => ModelState::Invalid,
            Err(_) => match fs::metadata(self.partial(m)) {
                Ok(p) => ModelState::Partial { bytes: p.len() },
                Err(_) => ModelState::NotInstalled,
            },
        }
    }

    /// Path to load, only if installed with the approved size. The worker
    /// re-verifies the hash while loading.
    pub fn installed_path(&self, m: &ApprovedModel) -> Option<PathBuf> {
        matches!(self.state(m), ModelState::Installed { .. }).then(|| self.path(m))
    }

    pub fn install(
        &self,
        m: &ApprovedModel,
        fetcher: &dyn Fetcher,
        progress: &mut dyn FnMut(Progress),
        cancel: &Cancel,
    ) -> Result<(), ModelError> {
        if matches!(self.state(m), ModelState::Installed { .. }) {
            return Ok(());
        }
        let partial = self.partial(m);
        download_to(m, &partial, fetcher, progress, cancel)?;
        self.commit(m, &partial)
    }

    /// Offline import of a user-selected regular file (docs/03 §6). The file
    /// is copied, never moved or linked, so later changes to the source
    /// cannot affect the installed model.
    pub fn import(
        &self,
        m: &ApprovedModel,
        source: &Path,
        progress: &mut dyn FnMut(Progress),
        cancel: &Cancel,
    ) -> Result<(), ModelError> {
        let meta = fs::metadata(source).map_err(|_| ModelError::NotAFile)?;
        if !meta.file_type().is_file() {
            return Err(ModelError::NotAFile);
        }
        if meta.len() != m.bytes {
            return Err(ModelError::WrongSize {
                got: meta.len(),
                expected: m.bytes,
            });
        }
        let mut src = File::open(source).map_err(|_| ModelError::NotAFile)?;
        let partial = self.partial(m);
        let result = (|| {
            let mut dst = private_create(&partial)?;
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 1 << 20];
            let mut done = 0u64;
            loop {
                if cancel.is_set() {
                    return Err(ModelError::Canceled);
                }
                let n = src.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                done += n as u64;
                if done > m.bytes {
                    return Err(ModelError::TooLarge); // file grew while copying
                }
                hasher.update(&buf[..n]);
                dst.write_all(&buf[..n])?;
                progress(Progress {
                    id: m.id,
                    done,
                    total: m.bytes,
                });
            }
            if done != m.bytes {
                return Err(ModelError::WrongSize {
                    got: done,
                    expected: m.bytes,
                });
            }
            if hex(&hasher.finalize()) != m.sha256 {
                return Err(ModelError::HashMismatch);
            }
            dst.sync_all()?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = fs::remove_file(&partial); // imports are not resumable
            return Err(e);
        }
        self.commit(m, &partial)
    }

    /// Full re-hash of an installed model. Records the result in the marker.
    pub fn verify(&self, m: &ApprovedModel, cancel: &Cancel) -> Result<(), ModelError> {
        let path = self.path(m);
        let meta = fs::symlink_metadata(&path).map_err(|_| ModelError::NotAFile)?;
        if !meta.file_type().is_file() {
            return Err(ModelError::NotAFile);
        }
        if meta.len() != m.bytes {
            return Err(ModelError::WrongSize {
                got: meta.len(),
                expected: m.bytes,
            });
        }
        let mut f = File::open(&path)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if cancel.is_set() {
                return Err(ModelError::Canceled);
            }
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        if hex(&hasher.finalize()) != m.sha256 {
            let _ = fs::remove_file(self.marker(m));
            return Err(ModelError::HashMismatch);
        }
        self.write_marker(m)
    }

    /// Removes the installed file, any partial download and the marker.
    /// The caller must ensure the model is not active.
    pub fn remove(&self, m: &ApprovedModel) -> Result<(), ModelError> {
        for p in [self.path(m), self.partial(m), self.marker(m)] {
            match fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        sync_dir(&self.dir);
        Ok(())
    }

    fn commit(&self, m: &ApprovedModel, partial: &Path) -> Result<(), ModelError> {
        fs::rename(partial, self.path(m))?;
        sync_dir(&self.dir);
        self.write_marker(m)
    }

    fn marker_value(meta: &fs::Metadata, m: &ApprovedModel) -> String {
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        format!("{} {} {}\n", m.sha256, meta.len(), mtime)
    }

    fn write_marker(&self, m: &ApprovedModel) -> Result<(), ModelError> {
        let meta = fs::symlink_metadata(self.path(m))?;
        let mut f = private_create(&self.marker(m))?;
        f.write_all(Self::marker_value(&meta, m).as_bytes())?;
        f.sync_all()?;
        Ok(())
    }

    fn marker_matches(&self, m: &ApprovedModel, meta: &fs::Metadata) -> bool {
        fs::read_to_string(self.marker(m)).is_ok_and(|s| s == Self::marker_value(meta, m))
    }
}

#[cfg(unix)]
fn private_create(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_create(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if dir.is_dir() {
        return Ok(());
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)
}

fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    let _ = dir;
}
