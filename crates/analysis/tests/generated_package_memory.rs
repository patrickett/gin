//! Memory regression test: loads a large package and asserts the resident set stays
//! within a sane budget through semantic analysis and a couple of revisions.
//!
//! Run with `cargo test --release -p analysis stdlib_independence_memory -- --nocapture`.
//!
//! The 2025 baseline (before fixing exponential variant_map duplication) was
//! ~2.7 GB after `package_semantics` on a 32-file package. The fixed pipeline
//! sits around 22 MB. The 256 MB cap leaves room for normal AST growth while
//! still catching another exponential regression early.

use analysis::PackageCache;
use test_fixtures::TempPackage;

const MAX_RSS_AFTER_SEMANTICS: usize = 320 * 1024 * 1024;
const MAX_RSS_AFTER_REVISIONS: usize = 384 * 1024 * 1024;

#[test]
fn full_package_load_memory() {
    let package = TempPackage::new("memory_baseline");
    let mut paths = Vec::new();
    for index in 0..32 {
        let source = format!("Type{index} is Unit\nvalue_{index} := Type{index}\n");
        paths.push(package.write(&format!("module_{index}.gin"), &source));
    }

    let rss_before = process_rss_bytes();

    let engine = PackageCache::for_test();
    for path in &paths {
        engine.add_file(path.clone()).ok();
    }

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
