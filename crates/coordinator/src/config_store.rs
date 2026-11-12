//! Atomic, user-private settings file (docs/04 §6).
//!
//! Writes go to a private temporary sibling that is fsynced and renamed over
//! the target, then the directory is fsynced. Corrupt files are moved aside to
//! a backup and defaults are used; future-schema files are left untouched and
//! the store becomes read-only until the user explicitly resets.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use dictation_domain::config::{Config, ConfigError};
use serde_json::Value;

const FILE_NAME: &str = "settings.json";
const MAX_FILE_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    Loaded,
    CreatedDefault,
    /// The previous file was unreadable; it was preserved at `backup`.
    RecoveredFromCorrupt {
        backup: PathBuf,
    },
    /// A newer app wrote this file. Defaults are used in memory and nothing is
    /// written until `reset` is called explicitly.
    FutureSchema {
        version: u32,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("settings i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error("settings were written by a newer version; reset required before saving")]
    ReadOnly,
}

pub struct ConfigStore {
    dir: PathBuf,
    current: Config,
    read_only: bool,
}

impl ConfigStore {
    pub fn open(dir: &Path) -> Result<(Self, LoadOutcome), StoreError> {
        create_private_dir(dir)?;
        let path = dir.join(FILE_NAME);
        let text = match read_bounded(&path) {
            Ok(t) => Some(t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Some(String::new()), // too large → corrupt
            Err(e) => return Err(e.into()),
        };
        let mut store = Self {
            dir: dir.to_path_buf(),
            current: Config::default(),
            read_only: false,
        };
        let outcome = match text {
            None => {
                store.write(&store.current.clone())?;
                LoadOutcome::CreatedDefault
            }
            Some(t) => match Config::from_json(&t) {
                Ok(cfg) => {
                    store.current = cfg;
                    LoadOutcome::Loaded
                }
                Err(ConfigError::FutureSchema(version)) => {
                    store.read_only = true;
                    LoadOutcome::FutureSchema { version }
                }
                Err(_) => {
                    let backup = store.backup_path();
                    fs::rename(&path, &backup)?;
                    store.write(&store.current.clone())?;
                    LoadOutcome::RecoveredFromCorrupt { backup }
                }
            },
        };
        Ok((store, outcome))
    }

    pub fn get(&self) -> &Config {
        &self.current
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Applies a validated merge-patch if `expected_revision` is current.
    /// Changes apply to the next capture; the caller decides when to re-read.
    pub fn update(&mut self, expected_revision: u64, patch: &Value) -> Result<&Config, StoreError> {
        if self.read_only {
            return Err(StoreError::ReadOnly);
        }
        let next = self.current.apply_patch(expected_revision, patch)?;
        self.write(&next)?;
        self.current = next;
        Ok(&self.current)
    }

    /// Explicit user reset: keeps a backup of whatever was there.
    pub fn reset(&mut self) -> Result<&Config, StoreError> {
        let path = self.dir.join(FILE_NAME);
        if path.exists() {
            fs::rename(&path, self.backup_path())?;
        }
        let next = Config {
            revision: self.current.revision + 1,
            ..Config::default()
        };
        self.write(&next)?;
        self.current = next;
        self.read_only = false;
        Ok(&self.current)
    }

    fn backup_path(&self) -> PathBuf {
        for n in 1u32.. {
            let p = self.dir.join(format!("{FILE_NAME}.bak-{n}"));
            if !p.exists() {
                return p;
            }
        }
        self.dir.join(format!("{FILE_NAME}.bak"))
    }

    fn write(&self, cfg: &Config) -> Result<(), StoreError> {
        cfg.validate()?;
        let tmp = self
            .dir
            .join(format!(".{FILE_NAME}.tmp-{}", std::process::id()));
        {
            let mut f = private_create(&tmp)?;
            f.write_all(cfg.to_json().as_bytes())?;
            f.write_all(b"\n")?;
            f.sync_all()?;
        }
        if let Err(e) = fs::rename(&tmp, self.dir.join(FILE_NAME)) {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        sync_dir(&self.dir);
        Ok(())
    }
}

fn read_bounded(path: &Path) -> std::io::Result<String> {
    let f = File::open(path)?;
    if f.metadata()?.len() > MAX_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "settings file too large",
        ));
    }
    let mut s = String::new();
    f.take(MAX_FILE_BYTES)
        .read_to_string(&mut s)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
    Ok(s)
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
    // Best effort: persists the rename on POSIX filesystems.
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    let _ = dir;
}
