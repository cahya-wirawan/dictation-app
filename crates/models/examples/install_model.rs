//! Installs an approved model over the real network into a directory.
//!     cargo run -p dictation-models --example install_model -- <dir> [model-id]
use std::time::Instant;

use dictation_models::download::HttpFetcher;
use dictation_models::{Cancel, ModelStore, find};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dir = args.next().ok_or("usage: install_model <dir> [model-id]")?;
    let id = args
        .next()
        .unwrap_or_else(|| "whisper-base-multilingual".into());
    let m = find(&id).ok_or("unknown model id")?;
    let store = ModelStore::open(std::path::Path::new(&dir))?;
    let fetcher = HttpFetcher::new(concat!("LocalDictation/", env!("CARGO_PKG_VERSION")));
    let t = Instant::now();
    let mut last = 0;
    store.install(
        m,
        &fetcher,
        &mut |p| {
            let pct = p.done * 100 / p.total;
            if pct >= last + 10 {
                last = pct;
                eprintln!("{pct}%");
            }
        },
        &Cancel::new(),
    )?;
    println!(
        "{} installed and verified in {:.1}s: {:?}",
        m.id,
        t.elapsed().as_secs_f32(),
        store.state(m)
    );
    Ok(())
}
