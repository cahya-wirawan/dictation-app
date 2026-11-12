//! M0 T0.6 development-only diagnostic. Delivers ONE fixed Unicode string via
//! clipboard + a single Cmd+V, then reads the focused field back to check the
//! receiving app actually contains it. No arbitrary text input, no IPC.
//! Compile-gated behind `m0-diagnostics`.
//!
//!     cargo run -p dictation-platform --features m0-diagnostics --example m0_paste_probe
//!
//! Then within 3 s focus an empty TextEdit document.

use std::time::Duration;

use dictation_platform::*;

const FIXED_TEXT: &str = "M0 paste probe: Grüße aus Köln — selamat pagi 👋 漢字テスト";

fn main() {
    println!("accessibility: {:?}", accessibility_permission());
    println!("focus the target field now; capturing in 3 s…");
    std::thread::sleep(Duration::from_secs(3));
    let start = match capture_target() {
        Ok(s) => s,
        Err(e) => return println!("BLOCKED: {e}"),
    };
    println!("target: {start:?}");
    let before = focused_text_value();

    // Simulate the inference interval, then validate again before any mutation.
    std::thread::sleep(Duration::from_millis(500));
    let check = check_target(&start);
    if !check.allows_auto_delivery() {
        return println!("BLOCKED before clipboard write: {check:?} (draft would be kept)");
    }
    if !wait_modifiers_released(MODIFIER_RELEASE_TIMEOUT) {
        return println!("BLOCKED: modifiers held (draft would be kept)");
    }
    let change = match clipboard_write_text(FIXED_TEXT) {
        Ok(c) => c,
        Err(e) => return println!("BLOCKED: {e}"),
    };
    println!("clipboard written (changeCount {change})");
    let check = check_target(&start);
    if !check.allows_auto_delivery() {
        return println!(
            "BLOCKED after clipboard write: {check:?}; clipboard now holds the probe text"
        );
    }
    let outcome = send_paste_chord();
    println!("dispatch: {outcome:?}");
    std::thread::sleep(Duration::from_millis(500));
    let after = focused_text_value();
    let received = after.as_deref().is_some_and(|a| a.contains(FIXED_TEXT));
    let duplicated = after.as_deref().is_some_and(|a| {
        a.matches(FIXED_TEXT).count()
            > before
                .as_deref()
                .map_or(0, |b| b.matches(FIXED_TEXT).count())
                + 1
    });
    println!("RECEIVED={received} DUPLICATED={duplicated}");
    if after.is_none() {
        println!("field value not readable via AX; inspect the document manually");
    }
}
