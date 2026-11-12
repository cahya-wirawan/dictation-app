//! Settings persistence: atomic writes, conflicts, corrupt/future files.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use dictation_coordinator::config_store::*;
use dictation_domain::config::{Config, ConfigError};
use serde_json::json;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("dictation-cfg-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn creates_defaults_privately_then_loads() {
    let dir = tmpdir("create");
    let (s, o) = ConfigStore::open(&dir).unwrap();
    assert_eq!(o, LoadOutcome::CreatedDefault);
    assert_eq!(s.get(), &Config::default());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("settings.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dmode, 0o700);
    }
    let (s2, o2) = ConfigStore::open(&dir).unwrap();
    assert_eq!(o2, LoadOutcome::Loaded);
    assert_eq!(s2.get(), s.get());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn update_persists_and_detects_conflicts() {
    let dir = tmpdir("update");
    let (mut s, _) = ConfigStore::open(&dir).unwrap();
    s.update(1, &json!({"audio": {"silence_ms": 1200}}))
        .unwrap();
    assert_eq!(s.get().revision, 2);
    // A second writer with a stale revision loses.
    let err = s
        .update(1, &json!({"audio": {"silence_ms": 900}}))
        .unwrap_err();
    assert!(matches!(
        err,
        StoreError::Config(ConfigError::Conflict {
            expected: 1,
            current: 2
        })
    ));
    // Invalid patch leaves the file untouched.
    assert!(s.update(2, &json!({"audio": {"silence_ms": 7}})).is_err());
    let (reloaded, _) = ConfigStore::open(&dir).unwrap();
    assert_eq!(reloaded.get().audio.silence_ms, 1200);
    assert_eq!(reloaded.get().revision, 2);
    // No temporary files left behind.
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter(|n| n.to_string_lossy().contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn corrupt_file_is_backed_up_and_replaced() {
    let dir = tmpdir("corrupt");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("settings.json"), b"{ truncated").unwrap();
    let (s, o) = ConfigStore::open(&dir).unwrap();
    let LoadOutcome::RecoveredFromCorrupt { backup } = o else {
        panic!("{o:?}")
    };
    assert_eq!(std::fs::read(&backup).unwrap(), b"{ truncated");
    assert_eq!(s.get(), &Config::default());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn oversized_file_is_treated_as_corrupt() {
    let dir = tmpdir("huge");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("settings.json"), vec![b' '; 2 * 1024 * 1024]).unwrap();
    let (_, o) = ConfigStore::open(&dir).unwrap();
    assert!(matches!(o, LoadOutcome::RecoveredFromCorrupt { .. }));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn future_schema_is_never_overwritten_until_reset() {
    let dir = tmpdir("future");
    std::fs::create_dir_all(&dir).unwrap();
    let future = r#"{"schema_version": 3, "new_feature": true}"#;
    std::fs::write(dir.join("settings.json"), future).unwrap();
    let (mut s, o) = ConfigStore::open(&dir).unwrap();
    assert_eq!(o, LoadOutcome::FutureSchema { version: 3 });
    assert!(s.is_read_only());
    assert!(matches!(
        s.update(1, &json!({"ui": {"show_hud": false}})),
        Err(StoreError::ReadOnly)
    ));
    assert_eq!(
        std::fs::read_to_string(dir.join("settings.json")).unwrap(),
        future
    );

    s.reset().unwrap();
    assert!(!s.is_read_only());
    let backups: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("settings.json.bak-")
        })
        .collect();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read_to_string(backups[0].path()).unwrap(), future);
    std::fs::remove_dir_all(dir).unwrap();
}
