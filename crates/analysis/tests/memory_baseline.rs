//! Memory regression test: loads `gin_core` and asserts the resident set stays
//! within a sane budget through semantic analysis and a couple of revisions.
//!
//! Run with `cargo test --release -p analysis memory_baseline -- --nocapture`.
//!
//! The 2025 baseline (before fixing exponential variant_map duplication) was
//! ~2.7 GB after `package_semantics` on a 32-file package. The fixed pipeline
//! sits around 22 MB. The 256 MB cap leaves room for normal AST growth while
//! still catching another exponential regression early.

use analysis::{CacheEngine, QueryEngine};
use crossbeam_channel::unbounded;
use std::path::PathBuf;

const MAX_RSS_AFTER_SEMANTICS: usize = 320 * 1024 * 1024;
const MAX_RSS_AFTER_REVISIONS: usize = 384 * 1024 * 1024;

#[test]
fn full_package_load_memory() {
    let modules_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("modules/gin_core");

    if !modules_dir.exists() {
        eprintln!("skip: gin_core modules not present at {modules_dir:?}");
        return;
    }

    let rss_before = process_rss_bytes();

    let (tx, _rx) = unbounded();
    let mut engine = CacheEngine::new(tx);
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in walkdir(&modules_dir) {
        if entry.extension().is_some_and(|e| e == "gin") {
            engine.add_file(entry.clone()).ok();
            paths.push(entry);
        }
    }
    paths.sort();

    let rss_after_load = process_rss_bytes();
    let _ = engine.package_semantics(&paths);
    let rss_after_semantics = process_rss_bytes();

    for _ in 0..5 {
        let _ = engine.package_semantics(&paths);
    }
    let rss_after_reruns = process_rss_bytes();

    let first_path = paths.first().cloned().unwrap();
    let original = std::fs::read_to_string(&first_path).unwrap();
    engine.set_contents(&first_path, format!("{original}\n-- modified\n"));
    let _ = engine.package_semantics(&paths);
    engine.set_contents(&first_path, original);
    let _ = engine.package_semantics(&paths);
    let rss_after_revisions = process_rss_bytes();

    println!("files loaded                  = {}", paths.len());
    println!("rss before load               = {:>12} bytes", rss_before);
    println!(
        "rss after add_file            = {:>12} bytes",
        rss_after_load
    );
    println!(
        "rss after package_semantics   = {:>12} bytes",
        rss_after_semantics
    );
    println!(
        "rss after 5 cached re-runs    = {:>12} bytes",
        rss_after_reruns
    );
    println!(
        "rss after 2 content revisions = {:>12} bytes",
        rss_after_revisions
    );

    let semantics_delta = rss_after_semantics.saturating_sub(rss_before);
    let revisions_delta = rss_after_revisions.saturating_sub(rss_before);

    assert!(
        semantics_delta < MAX_RSS_AFTER_SEMANTICS,
        "package_semantics grew RSS by {} bytes (>{} bytes); look for AST/ctx merge \
         duplication regressions in transform_package",
        semantics_delta,
        MAX_RSS_AFTER_SEMANTICS,
    );
    assert!(
        revisions_delta < MAX_RSS_AFTER_REVISIONS,
        "Two content revisions grew RSS by {} bytes (>{} bytes); a per-revision \
         leak has crept back into the salsa pipeline",
        revisions_delta,
        MAX_RSS_AFTER_REVISIONS,
    );
}

fn walkdir(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let p = entry.path();
            if p.is_dir() {
                out.extend(walkdir(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn process_rss_bytes() -> usize {
    use std::process::Command;
    let pid = std::process::id().to_string();
    let out = Command::new("ps").args(["-o", "rss=", "-p", &pid]).output();
    if let Ok(out) = out
        && let Ok(text) = std::str::from_utf8(&out.stdout)
        && let Ok(kb) = text.trim().parse::<usize>()
    {
        return kb * 1024;
    }
    0
}

#[cfg(not(target_os = "macos"))]
fn process_rss_bytes() -> usize {
    // Linux: /proc/self/status VmRSS
    if let Ok(text) = std::fs::read_to_string("/proc/self/status") {
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("VmRSS:") {
                let kb: usize = rest
                    .trim()
                    .split_whitespace()
                    .next()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                return kb * 1024;
            }
        }
    }
    0
}
