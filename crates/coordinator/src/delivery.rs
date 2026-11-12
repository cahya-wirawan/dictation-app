//! Automatic delivery (docs/05 §4–5, work packages C1–C2).
//!
//! [`PlatformDelivery`] owns native target tokens on one dedicated thread
//! (macOS Accessibility references are not `Send`) and runs the clipboard
//! transaction. The reducer decides; this adapter only executes:
//!
//! 1. `capture_target` at the start press: an editable, non-secure,
//!    non-excluded, non-self target, or the session is copy-only.
//! 2. `prepare`: re-validate the target, wait (≤1500 ms) for the user to release
//!    the physical modifiers (never synthesizing releases), write plain text to
//!    the clipboard, re-validate again. Any failure becomes a draft.
//! 3. `dispatch`: runs only after the reducer committed the ledger, and posts
//!    exactly one paste chord. API success is reported as `Sent`, not as proof
//!    that the destination accepted the text.
//!
//! `release` marks the session canceled immediately, so a `prepare` waiting on
//! modifiers stops before it touches the clipboard.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use dictation_domain::session::{AppError, DeliveryMode, DispatchResult};
use dictation_domain::{Generation, SessionId};
use dictation_platform::{DispatchOutcome, FieldClass, TargetCheck};

use crate::actor::{AdapterEvent, DeliveryAdapter, Msg};

/// docs/05 §4 step 5.
pub const MODIFIER_TIMEOUT: Duration = Duration::from_millis(1500);

/// Terminal emulators are excluded from automatic delivery by default
/// (docs/05 §5): a pasted newline can execute a command. Matched by native
/// app identity (bundle id), never by window title.
pub const DEFAULT_EXCLUDED_APPS: &[&str] = &[
    "com.apple.Terminal",
    "com.googlecode.iterm2",
    "dev.warp.Warp-Stable",
    "io.alacritty",
    "org.alacritty",
    "net.kovidgoyal.kitty",
    "com.mitchellh.ghostty",
    "co.zeit.hyper",
    "com.github.wez.wezterm",
    "org.tabby",
];

/// Native operations the transaction needs. `Token` may be `!Send`: a
/// platform instance is created and used only on the delivery thread.
pub trait TargetPlatform {
    type Token;
    fn capture(&mut self) -> Result<Self::Token, AppError>;
    fn field(&self, t: &Self::Token) -> FieldClass;
    fn app_id(&self, t: &Self::Token) -> Option<String>;
    fn check(&mut self, t: &Self::Token) -> TargetCheck;
    /// Waits for physical modifiers to be released; false on timeout or if
    /// the state cannot be observed.
    fn modifiers_released(&mut self, timeout: Duration) -> bool;
    fn write_clipboard(&mut self, text: &str) -> Result<(), AppError>;
    fn send_paste(&mut self) -> Result<DispatchOutcome, AppError>;
}

/// The real platform via `dictation-platform`.
pub struct NativePlatform;

impl TargetPlatform for NativePlatform {
    type Token = dictation_platform::TargetSnapshot;

    fn capture(&mut self) -> Result<Self::Token, AppError> {
        dictation_platform::capture_target().map_err(|_| AppError::PasteUnsupported)
    }
    fn field(&self, t: &Self::Token) -> FieldClass {
        t.field
    }
    #[cfg(target_os = "macos")]
    fn app_id(&self, t: &Self::Token) -> Option<String> {
        t.app.bundle_id.clone()
    }
    #[cfg(not(target_os = "macos"))]
    fn app_id(&self, t: &Self::Token) -> Option<String> {
        t.app.as_ref().and_then(|a| a.bundle_id.clone())
    }
    fn check(&mut self, t: &Self::Token) -> TargetCheck {
        dictation_platform::check_target(t)
    }
    fn modifiers_released(&mut self, timeout: Duration) -> bool {
        dictation_platform::wait_modifiers_released(timeout)
    }
    fn write_clipboard(&mut self, text: &str) -> Result<(), AppError> {
        dictation_platform::clipboard_write_text(text)
            .map(|_| ())
            .map_err(|_| AppError::ClipboardBusy)
    }
    fn send_paste(&mut self) -> Result<DispatchOutcome, AppError> {
        dictation_platform::send_paste_chord().map_err(|_| AppError::PasteUnsupported)
    }
}

/// Maps a failed re-validation to the reason shown to the user.
pub fn check_error(c: TargetCheck) -> AppError {
    match c {
        TargetCheck::AppChanged | TargetCheck::WindowChanged | TargetCheck::FieldChanged => {
            AppError::TargetChanged
        }
        TargetCheck::Same => AppError::TargetChanged, // not an error; unreachable by callers
        TargetCheck::OwnApp
        | TargetCheck::Secure
        | TargetCheck::NotEditable
        | TargetCheck::Unknown => AppError::PasteUnsupported,
    }
}

enum Req {
    Capture {
        id: SessionId,
        reply: Sender<bool>,
    },
    Prepare {
        id: SessionId,
        generation: Generation,
        text: String,
        events: Sender<Msg>,
    },
    Dispatch {
        id: SessionId,
        generation: Generation,
        events: Sender<Msg>,
    },
    Release {
        id: SessionId,
    },
    Copy {
        id: SessionId,
        revision: u32,
        text: String,
        events: Sender<Msg>,
    },
}

pub type SharedExclusions = Arc<Mutex<Vec<String>>>;

pub struct PlatformDelivery {
    tx: Sender<Req>,
    canceled: Arc<Mutex<HashSet<SessionId>>>,
}

impl PlatformDelivery {
    /// `make` builds the platform on the delivery thread. `excluded` holds
    /// user-configured bundle ids in addition to [`DEFAULT_EXCLUDED_APPS`].
    pub fn spawn<P, F>(make: F, excluded: SharedExclusions) -> Self
    where
        P: TargetPlatform + 'static,
        F: FnOnce() -> P + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let canceled = Arc::new(Mutex::new(HashSet::new()));
        let flags = Arc::clone(&canceled);
        let _ = thread::Builder::new()
            .name("delivery".into())
            .spawn(move || run(make(), &rx, &flags, &excluded));
        Self { tx, canceled }
    }

    fn is_canceled(&self, id: SessionId) -> bool {
        self.canceled
            .lock()
            .map(|c| c.contains(&id))
            .unwrap_or(true)
    }
}

impl DeliveryAdapter for PlatformDelivery {
    fn capture_target(&mut self, id: SessionId) -> bool {
        if let Ok(mut c) = self.canceled.lock() {
            c.remove(&id);
        }
        let (reply, rx) = mpsc::channel();
        if self.tx.send(Req::Capture { id, reply }).is_err() {
            return false;
        }
        // Capture happens at the press; a stuck platform call must not hang the actor.
        rx.recv_timeout(Duration::from_millis(500)).unwrap_or(false) && !self.is_canceled(id)
    }

    fn prepare(
        &mut self,
        id: SessionId,
        generation: Generation,
        _mode: DeliveryMode,
        text: String,
        events: Sender<Msg>,
    ) {
        let _ = self.tx.send(Req::Prepare {
            id,
            generation,
            text,
            events,
        });
    }

    fn dispatch(&mut self, id: SessionId, generation: Generation, events: Sender<Msg>) {
        let _ = self.tx.send(Req::Dispatch {
            id,
            generation,
            events,
        });
    }

    fn release(&mut self, id: SessionId) {
        if let Ok(mut c) = self.canceled.lock() {
            c.insert(id);
        }
        let _ = self.tx.send(Req::Release { id });
    }

    fn copy(&mut self, id: SessionId, revision: u32, text: String, events: Sender<Msg>) {
        let _ = self.tx.send(Req::Copy {
            id,
            revision,
            text,
            events,
        });
    }
}

fn excluded_app<P: TargetPlatform>(p: &P, t: &P::Token, user: &SharedExclusions) -> bool {
    let Some(app) = p.app_id(t) else { return false };
    DEFAULT_EXCLUDED_APPS
        .iter()
        .any(|e| e.eq_ignore_ascii_case(&app))
        || user
            .lock()
            .map(|u| u.iter().any(|e| e.eq_ignore_ascii_case(&app)))
            .unwrap_or(true)
}

fn run<P: TargetPlatform>(
    mut p: P,
    rx: &Receiver<Req>,
    canceled: &Mutex<HashSet<SessionId>>,
    excluded: &SharedExclusions,
) {
    let mut tokens: HashMap<SessionId, P::Token> = HashMap::new();
    let is_canceled = |id: SessionId| canceled.lock().map(|c| c.contains(&id)).unwrap_or(true);
    while let Ok(req) = rx.recv() {
        match req {
            Req::Capture { id, reply } => {
                tokens.remove(&id);
                let ok = match p.capture() {
                    Ok(t) => {
                        let usable = p.field(&t) == FieldClass::Editable
                            && !excluded_app(&p, &t, excluded)
                            && p.check(&t) == TargetCheck::Same;
                        if usable {
                            tokens.insert(id, t);
                        }
                        usable
                    }
                    Err(_) => false,
                };
                let _ = reply.send(ok);
            }
            Req::Prepare {
                id,
                generation,
                text,
                events,
            } => {
                let result = (|| {
                    let t = tokens.get(&id).ok_or(AppError::PasteUnsupported)?;
                    let c = p.check(t);
                    if c != TargetCheck::Same {
                        return Err(check_error(c));
                    }
                    if !p.modifiers_released(MODIFIER_TIMEOUT) {
                        return Err(AppError::ModifiersHeld);
                    }
                    if is_canceled(id) {
                        return Err(AppError::StaleRequest); // reducer already moved on
                    }
                    p.write_clipboard(&text)?;
                    // Clipboard mutation cannot be atomic with focus: recheck.
                    let c = p.check(t);
                    if c != TargetCheck::Same {
                        return Err(check_error(c));
                    }
                    Ok(())
                })();
                let _ = events.send(Msg::Adapter(AdapterEvent::DeliveryPrepared {
                    id,
                    generation,
                    result,
                }));
            }
            Req::Dispatch {
                id,
                generation,
                events,
            } => {
                // The reducer has committed: send exactly once, even if a
                // cancel arrived after commit (it cannot retract the send).
                let result = match p.send_paste() {
                    Ok(DispatchOutcome::Sent) => DispatchResult::Sent,
                    Ok(DispatchOutcome::Uncertain) | Err(_) => DispatchResult::Uncertain,
                };
                tokens.remove(&id);
                let _ = events.send(Msg::Adapter(AdapterEvent::DispatchFinished {
                    id,
                    generation,
                    result,
                }));
            }
            Req::Release { id } => {
                tokens.remove(&id);
            }
            Req::Copy {
                id,
                revision,
                text,
                events,
            } => {
                let ok = p.write_clipboard(&text).is_ok();
                let _ = events.send(Msg::Adapter(AdapterEvent::CopyFinished {
                    id,
                    revision,
                    ok,
                }));
            }
        }
    }
}
