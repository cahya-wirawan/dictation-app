//! Persistent configuration schema (docs/04 §6), validation, merge-patch and
//! migration. Pure: no file I/O (see `dictation-coordinator::config_store`).
//!
//! Never persisted here: target tokens, transcripts, PCM, clipboard snapshots
//! or permission grants.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: u32 = 1;
const MAX_EXCLUDED_APPS: usize = 64;
const MAX_STRING: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub revision: u64,
    pub audio: AudioConfig,
    pub transcription: TranscriptionConfig,
    pub delivery: DeliveryConfig,
    pub shortcuts: ShortcutConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioConfig {
    /// `None`: confirm the system default during setup. Otherwise an opaque
    /// device id; a missing device requires reselection (never substitution).
    pub device_id: Option<String>,
    pub silence_ms: u32,
    pub no_speech_timeout_ms: u32,
    pub max_capture_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TranscriptionConfig {
    pub model_id: String,
    /// `"auto"` or a 2–3 letter lowercase code.
    pub language: String,
    pub backend: Backend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    Auto,
    Cpu,
    Metal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryConfig {
    pub mode: DeliveryModeSetting,
    pub clipboard_policy: ClipboardPolicy,
    /// Native app identities (bundle ids), never window titles.
    pub excluded_apps: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryModeSetting {
    AutoPaste,
    CopyOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardPolicy {
    LeaveText,
    /// P1; adapters that cannot snapshot all formats report it unavailable.
    Preserve,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShortcutConfig {
    pub toggle: String,
    pub cancel: String,
    pub paste_draft: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    pub show_hud: bool,
    pub launch_at_login: bool,
    pub locale: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            revision: 1,
            audio: AudioConfig {
                device_id: None,
                silence_ms: 1000,
                no_speech_timeout_ms: 8000,
                max_capture_ms: 120_000,
            },
            transcription: TranscriptionConfig {
                model_id: "whisper-base-multilingual".into(),
                language: "auto".into(),
                backend: Backend::Auto,
            },
            delivery: DeliveryConfig {
                mode: DeliveryModeSetting::AutoPaste,
                clipboard_policy: ClipboardPolicy::LeaveText,
                excluded_apps: vec![],
            },
            shortcuts: ShortcutConfig {
                toggle: "Ctrl+Alt+Space".into(),
                cancel: "Ctrl+Alt+Escape".into(),
                paste_draft: None,
            },
            ui: UiConfig {
                show_hud: true,
                launch_at_login: false,
                locale: "en".into(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("config is not valid JSON or does not match the schema: {0}")]
    Corrupt(String),
    #[error("config schema {0} is newer than this app supports ({SCHEMA_VERSION})")]
    FutureSchema(u32),
    #[error("invalid setting {field}: {reason}")]
    Invalid { field: &'static str, reason: String },
    #[error("settings changed elsewhere (expected revision {expected}, current {current})")]
    Conflict { expected: u64, current: u64 },
    #[error("patch may not change {0}")]
    Immutable(&'static str),
}

fn invalid(field: &'static str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        field,
        reason: reason.into(),
    }
}

fn check_str(field: &'static str, s: &str) -> Result<(), ConfigError> {
    if s.is_empty() || s.len() > MAX_STRING || s.chars().any(char::is_control) {
        return Err(invalid(field, "must be 1-256 printable characters"));
    }
    Ok(())
}

impl Config {
    /// Range checks from docs/01 §5 (re-validated in Rust regardless of UI).
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(invalid("schema_version", "unsupported"));
        }
        let a = &self.audio;
        if !(500..=2500).contains(&a.silence_ms) || !a.silence_ms.is_multiple_of(100) {
            return Err(invalid("audio.silence_ms", "500-2500 in steps of 100"));
        }
        if !(3000..=15_000).contains(&a.no_speech_timeout_ms) {
            return Err(invalid("audio.no_speech_timeout_ms", "3000-15000"));
        }
        if a.max_capture_ms != 120_000 {
            return Err(invalid("audio.max_capture_ms", "fixed at 120000 in v1"));
        }
        if let Some(d) = &a.device_id {
            check_str("audio.device_id", d)?;
        }
        let t = &self.transcription;
        check_str("transcription.model_id", &t.model_id)?;
        let lang_ok = t.language == "auto"
            || ((2..=3).contains(&t.language.len())
                && t.language.bytes().all(|b| b.is_ascii_lowercase()));
        if !lang_ok {
            return Err(invalid(
                "transcription.language",
                "\"auto\" or a 2-3 letter lowercase code",
            ));
        }
        if self.delivery.excluded_apps.len() > MAX_EXCLUDED_APPS {
            return Err(invalid("delivery.excluded_apps", "at most 64 entries"));
        }
        for app in &self.delivery.excluded_apps {
            check_str("delivery.excluded_apps", app)?;
        }
        check_str("shortcuts.toggle", &self.shortcuts.toggle)?;
        check_str("shortcuts.cancel", &self.shortcuts.cancel)?;
        if let Some(p) = &self.shortcuts.paste_draft {
            check_str("shortcuts.paste_draft", p)?;
        }
        let distinct = [
            Some(&self.shortcuts.toggle),
            Some(&self.shortcuts.cancel),
            self.shortcuts.paste_draft.as_ref(),
        ];
        for (i, a) in distinct.iter().enumerate() {
            for b in &distinct[i + 1..] {
                if let (Some(a), Some(b)) = (a, b)
                    && a.eq_ignore_ascii_case(b)
                {
                    return Err(invalid("shortcuts", "chords must be distinct"));
                }
            }
        }
        let loc = &self.ui.locale;
        if loc.is_empty()
            || loc.len() > 35
            || !loc.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(invalid("ui.locale", "BCP 47 tag"));
        }
        Ok(())
    }

    /// Parses stored JSON, migrating older schemas forward. Future schemas are
    /// refused so a downgrade never destroys newer settings.
    pub fn from_json(text: &str) -> Result<Self, ConfigError> {
        let value: Value =
            serde_json::from_str(text).map_err(|e| ConfigError::Corrupt(e.to_string()))?;
        let version = value
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| ConfigError::Corrupt("missing schema_version".into()))?;
        let version = u32::try_from(version).map_err(|_| ConfigError::FutureSchema(u32::MAX))?;
        if version > SCHEMA_VERSION {
            return Err(ConfigError::FutureSchema(version));
        }
        let migrated = migrate(value, version)?;
        let cfg: Config =
            serde_json::from_value(migrated).map_err(|e| ConfigError::Corrupt(e.to_string()))?;
        cfg.validate()
            .map_err(|e| ConfigError::Corrupt(e.to_string()))?;
        Ok(cfg)
    }

    pub fn to_json(&self) -> String {
        // Serialization of plain data cannot fail.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// RFC 7386 JSON merge-patch with optimistic concurrency. The result is
    /// fully re-validated; `schema_version` and `revision` are not patchable.
    pub fn apply_patch(&self, expected_revision: u64, patch: &Value) -> Result<Self, ConfigError> {
        if expected_revision != self.revision {
            return Err(ConfigError::Conflict {
                expected: expected_revision,
                current: self.revision,
            });
        }
        let Value::Object(map) = patch else {
            return Err(ConfigError::Corrupt("patch must be a JSON object".into()));
        };
        for key in ["schema_version", "revision"] {
            if map.contains_key(key) {
                return Err(ConfigError::Immutable(if key == "revision" {
                    "revision"
                } else {
                    "schema_version"
                }));
            }
        }
        let mut doc =
            serde_json::to_value(self).map_err(|e| ConfigError::Corrupt(e.to_string()))?;
        merge_patch(&mut doc, patch);
        let mut next: Config =
            serde_json::from_value(doc).map_err(|e| ConfigError::Corrupt(e.to_string()))?;
        next.validate()?;
        next.revision = self.revision + 1;
        Ok(next)
    }
}

/// Chain of schema migrations. v1 is current; future versions add steps here.
fn migrate(value: Value, from: u32) -> Result<Value, ConfigError> {
    match from {
        SCHEMA_VERSION => Ok(value),
        other => Err(ConfigError::Corrupt(format!(
            "unknown schema_version {other}"
        ))),
    }
}

fn merge_patch(target: &mut Value, patch: &Value) {
    match patch {
        Value::Object(p) => {
            if !target.is_object() {
                *target = Value::Object(serde_json::Map::new());
            }
            if let Value::Object(t) = target {
                for (k, v) in p {
                    if v.is_null() {
                        // For nullable fields (device_id, paste_draft) null is a
                        // value, not a deletion; schema deserialization rejects
                        // removal of required fields anyway.
                        t.insert(k.clone(), Value::Null);
                    } else {
                        merge_patch(t.entry(k.clone()).or_insert(Value::Null), v);
                    }
                }
            }
        }
        other => *target = other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_valid_and_match_spec() {
        let c = Config::default();
        c.validate().unwrap();
        assert_eq!(
            (
                c.audio.silence_ms,
                c.audio.no_speech_timeout_ms,
                c.audio.max_capture_ms
            ),
            (1000, 8000, 120_000)
        );
        assert_eq!(c.shortcuts.toggle, "Ctrl+Alt+Space");
        assert_eq!(c.delivery.clipboard_policy, ClipboardPolicy::LeaveText);
    }

    #[test]
    fn json_roundtrip_matches_documented_shape() {
        let c = Config::default();
        let v: Value = serde_json::from_str(&c.to_json()).unwrap();
        assert_eq!(v["delivery"]["mode"], "auto_paste");
        assert_eq!(v["transcription"]["backend"], "auto");
        assert_eq!(v["audio"]["device_id"], Value::Null);
        assert_eq!(Config::from_json(&c.to_json()).unwrap(), c);
    }

    #[test]
    fn patch_validates_and_bumps_revision() {
        let c = Config::default();
        let n = c
            .apply_patch(
                1,
                &json!({"audio": {"silence_ms": 1500}, "transcription": {"language": "de"}}),
            )
            .unwrap();
        assert_eq!(
            (
                n.audio.silence_ms,
                n.transcription.language.as_str(),
                n.revision
            ),
            (1500, "de", 2)
        );
        assert_eq!(n.audio.no_speech_timeout_ms, 8000, "untouched fields kept");

        for (patch, field) in [
            (json!({"audio": {"silence_ms": 1550}}), "audio.silence_ms"),
            (json!({"audio": {"silence_ms": 400}}), "audio.silence_ms"),
            (
                json!({"audio": {"no_speech_timeout_ms": 20000}}),
                "audio.no_speech_timeout_ms",
            ),
            (
                json!({"audio": {"max_capture_ms": 60000}}),
                "audio.max_capture_ms",
            ),
            (
                json!({"transcription": {"language": "German"}}),
                "transcription.language",
            ),
            (
                json!({"shortcuts": {"cancel": "ctrl+alt+space"}}),
                "shortcuts",
            ),
            (json!({"ui": {"locale": "en_US;rm -rf"}}), "ui.locale"),
        ] {
            assert!(
                matches!(c.apply_patch(1, &patch), Err(ConfigError::Invalid { field: f, .. }) if f == field),
                "{patch}"
            );
        }
    }

    #[test]
    fn patch_rejects_conflicts_unknown_fields_and_immutables() {
        let c = Config::default();
        assert_eq!(
            c.apply_patch(7, &json!({})),
            Err(ConfigError::Conflict {
                expected: 7,
                current: 1
            })
        );
        assert!(matches!(
            c.apply_patch(1, &json!({"audio": {"gain": 3}})),
            Err(ConfigError::Corrupt(_))
        ));
        assert!(matches!(
            c.apply_patch(1, &json!({"telemetry": true})),
            Err(ConfigError::Corrupt(_))
        ));
        assert_eq!(
            c.apply_patch(1, &json!({"revision": 9})),
            Err(ConfigError::Immutable("revision"))
        );
        assert!(matches!(
            c.apply_patch(1, &json!({"audio": null})),
            Err(ConfigError::Corrupt(_))
        ));
        assert!(matches!(
            c.apply_patch(1, &json!([1])),
            Err(ConfigError::Corrupt(_))
        ));
    }

    #[test]
    fn nullable_fields_can_be_cleared() {
        let c = Config::default()
            .apply_patch(1, &json!({"audio": {"device_id": "coreaudio:mic-1"}}))
            .unwrap();
        let c = c
            .apply_patch(2, &json!({"audio": {"device_id": null}}))
            .unwrap();
        assert_eq!(c.audio.device_id, None);
    }

    #[test]
    fn future_and_corrupt_schemas() {
        assert_eq!(
            Config::from_json(r#"{"schema_version": 2}"#),
            Err(ConfigError::FutureSchema(2))
        );
        assert!(matches!(
            Config::from_json("{not json"),
            Err(ConfigError::Corrupt(_))
        ));
        assert!(matches!(
            Config::from_json(r#"{"revision": 1}"#),
            Err(ConfigError::Corrupt(_))
        ));
        let mut bad: Value = serde_json::from_str(&Config::default().to_json()).unwrap();
        bad["audio"]["silence_ms"] = json!(5);
        assert!(matches!(
            Config::from_json(&bad.to_string()),
            Err(ConfigError::Corrupt(_))
        ));
    }
}
