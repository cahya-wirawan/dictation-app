//! M0 T0.5: prints the frontmost app/window/focused-field identity once per
//! second and whether it changed from the first sample. Click between apps,
//! windows and fields (TextEdit, a browser textarea, a password field) and
//! record the output in docs/IMPLEMENTATION_STATUS.md.
//!
//!     cargo run -p dictation-platform --example m0_target_probe

use std::time::Duration;

fn main() {
    println!(
        "accessibility: {:?}",
        dictation_platform::accessibility_permission()
    );
    let start = match dictation_platform::capture_target() {
        Ok(s) => s,
        Err(e) => {
            println!("cannot capture target: {e}");
            return;
        }
    };
    println!("start: {start:?}");
    for _ in 0..60 {
        std::thread::sleep(Duration::from_secs(1));
        let now = dictation_platform::capture_target();
        let check = dictation_platform::check_target(&start);
        println!("{check:?}  now={now:?}");
    }
}
