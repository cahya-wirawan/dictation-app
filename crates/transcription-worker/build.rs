//! Builds the pinned whisper.cpp submodule as static libraries (CPU backend,
//! OpenMP off, optional Metal) and compiles the C shim against it.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The vendored submodule pin (docs/IMPLEMENTATION_STATUS.md).
const PINNED_COMMIT: &str = "d1be6fde11ac6e0407606b4e42fe72d34add8037";
const PINNED_TAG: &str = "v1.9.5";

#[allow(clippy::panic, clippy::expect_used)]
fn main() {
    let manifest =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let src = manifest.join("../../vendor/whisper.cpp");
    if !src.join("CMakeLists.txt").exists() {
        panic!("vendor/whisper.cpp is missing: run `git submodule update --init --recursive`");
    }
    println!("cargo::rerun-if-changed=shim.c");
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=../../vendor/whisper.cpp/include/whisper.h");
    println!("cargo::rerun-if-env-changed=DICTATION_GGML_NATIVE");
    println!("cargo::rerun-if-env-changed=MACOSX_DEPLOYMENT_TARGET");

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    // Metal exists only on Apple targets; the feature is a no-op elsewhere.
    let metal = env::var_os("CARGO_FEATURE_METAL").is_some() && target_os == "macos";
    // GGML_NATIVE=ON tunes for the build machine; release/distributed builds
    // must set DICTATION_GGML_NATIVE=OFF so the binary runs on every target CPU.
    let native = env::var("DICTATION_GGML_NATIVE").unwrap_or_else(|_| "ON".into());

    let mut cfg = cmake::Config::new(&src);
    if target_os == "macos" {
        // Must match the deployment target rustc links with (.cargo/config.toml),
        // otherwise ld warns that objects target a newer macOS than the binary.
        let min = env::var("MACOSX_DEPLOYMENT_TARGET").unwrap_or_else(|_| "13.0".into());
        cfg.define("CMAKE_OSX_DEPLOYMENT_TARGET", &min);
    }
    if env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // cc's defaults for a debug Rust profile include `-Od`, which MSVC
        // applies after CMake's Release `/O2` and leaves ggml unoptimized
        // (60 s+ for the 11 s fixture). CMake's own Release flags (/O2 /MD)
        // already match Rust's CRT.
        cfg.no_default_flags(true);
    }
    if native == "OFF" && target_arch == "x86_64" {
        // Portable x86 baseline = x86-64-v3 (Haswell, 2013+). Without this ggml
        // falls back to SSE-only code, ~11x slower on the JFK fixture.
        for opt in [
            "GGML_AVX",
            "GGML_AVX2",
            "GGML_FMA",
            "GGML_F16C",
            "GGML_BMI2",
        ] {
            cfg.define(opt, "ON");
        }
    }
    let dst = cfg
        .profile("Release")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("WHISPER_BUILD_TESTS", "OFF")
        .define("WHISPER_BUILD_EXAMPLES", "OFF")
        .define("WHISPER_BUILD_SERVER", "OFF")
        .define("WHISPER_SDL2", "OFF")
        .define("WHISPER_CURL", "OFF")
        .define("GGML_OPENMP", "OFF")
        .define("GGML_BLAS", "OFF")
        .define("GGML_NATIVE", &native)
        .define("GGML_METAL", if metal { "ON" } else { "OFF" })
        .define("GGML_METAL_EMBED_LIBRARY", if metal { "ON" } else { "OFF" })
        .build();

    let mut found = Vec::new();
    for dir in ["lib", "lib64"] {
        let d = dst.join(dir);
        if d.is_dir() {
            println!("cargo::rustc-link-search=native={}", d.display());
            found.extend(static_libs(&d));
        }
    }
    // GNU ld resolves static archives left to right: dependents first.
    for lib in [
        "whisper",
        "ggml",
        "ggml-cpu",
        "ggml-metal",
        "ggml-blas",
        "ggml-base",
    ] {
        if found.iter().any(|f| f == lib) {
            println!("cargo::rustc-link-lib=static={lib}");
        }
    }
    if !found.iter().any(|f| f == "whisper") {
        panic!(
            "whisper.cpp build produced no whisper static library under {}",
            dst.display()
        );
    }
    match target_os.as_str() {
        "macos" => {
            println!("cargo::rustc-link-lib=c++");
            println!("cargo::rustc-link-lib=framework=Foundation");
            // ggml-cpu uses vDSP from Accelerate (GGML_ACCELERATE, default ON).
            println!("cargo::rustc-link-lib=framework=Accelerate");
            if metal {
                println!("cargo::rustc-link-lib=framework=Metal");
                println!("cargo::rustc-link-lib=framework=MetalKit");
            }
        }
        // ggml-cpu reads the CPU name from the registry.
        "windows" => println!("cargo::rustc-link-lib=advapi32"),
        _ => println!("cargo::rustc-link-lib=stdc++"),
    }

    cc::Build::new()
        .file("shim.c")
        .include(src.join("include"))
        .include(src.join("ggml/include"))
        .warnings(true)
        .compile("dictation_whisper_shim");

    let revision = git(&src, &["rev-parse", "HEAD"]);
    let mut tag = git(&src, &["describe", "--tags", "--always"]);
    // Shallow CI checkouts carry no tags. Name the tag only for the exact
    // pinned commit; any other commit keeps its bare hash.
    if !tag.starts_with('v') && revision == PINNED_COMMIT {
        tag = PINNED_TAG.to_owned();
    }
    println!("cargo::rustc-env=WHISPER_CPP_REVISION={revision}");
    println!("cargo::rustc-env=WHISPER_CPP_TAG={tag}");
    println!("cargo::rustc-env=DICTATION_GGML_NATIVE={native}");
}

fn static_libs(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    rd.filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            // Unix/MinGW: libNAME.a; MSVC: NAME.lib.
            let stem = name
                .strip_prefix("lib")
                .and_then(|n| n.strip_suffix(".a"))
                .or_else(|| name.strip_suffix(".lib"))?;
            Some(stem.to_owned())
        })
        .collect()
}

fn git(dir: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "unknown".into())
}
