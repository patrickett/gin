#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use analysis::PackageCache;
use resolve::GinPackageExt;

#[global_allocator]
static ALLOC: TrackingAlloc = TrackingAlloc {
    current: AtomicUsize::new(0),
    peak: AtomicUsize::new(0),
};

struct TrackingAlloc {
    current: AtomicUsize,
    peak: AtomicUsize,
}

unsafe impl GlobalAlloc for TrackingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let size = layout.size();
            let new = self.current.fetch_add(size, Ordering::Relaxed) + size;
            let mut observed = self.peak.load(Ordering::Relaxed);
            while new > observed {
                match self.peak.compare_exchange(
                    observed,
                    new,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(actual) => observed = actual,
                }
            }
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if !ptr.is_null() {
            self.current.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.dealloc(ptr, layout) };
    }
}

fn report(label: &str) {
    let current = ALLOC.current.load(Ordering::Relaxed);
    let peak = ALLOC.peak.load(Ordering::Relaxed);
    println!("{label}: current_bytes={current} peak_bytes={peak}");
}

fn report_cache(cache: &PackageCache, label: &str) {
    let (files, packages, module_caches, import_graphs) = cache.cache_counts();
    println!(
        "{label}: cache_entries files={files} packages={packages} module_caches={module_caches} import_graphs={import_graphs}"
    );
}

fn load_root_cache(cache: &PackageCache, root: &Path) -> Vec<PathBuf> {
    let mut paths = root.collect_gin_files();
    paths.sort();
    for path in &paths {
        let _ = cache.add_file(path.clone());
    }
    paths
}

fn main() {
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "modules/gin_core".to_string());

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let mut root_path = PathBuf::from(manifest_dir);
    root_path.pop();
    root_path.pop();
    let root = root_path.join(root);

    println!("ginlsp memory probe");
    println!("repo_root={}", root_path.display());

    let (_tx, _rx) = crossbeam_channel::unbounded();
    let cache = PackageCache::new(_tx);

    report("start");
    report_cache(&cache, "start");

    let mut paths = load_root_cache(&cache, &root);
    println!("registered_files={}", paths.len());
    report("after_add_file");
    report_cache(&cache, "after_add_file");

    let t = Instant::now();
    let diags = cache.all_diagnostics(&paths);
    let elapsed = t.elapsed();
    println!("all_diagnostics_len={}", diags.len());
    println!(
        "diagnostic_compute_ms={:.3}",
        elapsed.as_secs_f64() * 1000.0
    );
    report("after_all_diagnostics");
    report_cache(&cache, "after_all_diagnostics");

    for _ in 0..5 {
        let _ = cache.all_diagnostics(&paths);
    }
    report("after_steady_state_diagnostics");
    report_cache(&cache, "after_steady_state_diagnostics");

    if let Some(first) = paths.first() {
        let mut source = std::fs::read_to_string(first).unwrap_or_default();
        source.push('\n');
        cache.set_contents(first, source);
        report("after_set_contents_spike_mutation");
        report_cache(&cache, "after_set_contents_spike_mutation");

        let _ = cache.all_diagnostics(&paths);
        report("after_recompute_after_mutation");
        report_cache(&cache, "after_recompute_after_mutation");
    }

    println!("starting_reload_churn");
    for i in 0..20 {
        let _ = cache.all_diagnostics(&paths);
        for path in &paths {
            let _ = cache.evict_file(path);
        }
        if i % 5 == 0 || i == 19 {
            report(&format!("after_reload_cycle_{i}"));
            report_cache(&cache, &format!("after_reload_cycle_{i}"));
        }
        paths = load_root_cache(&cache, &root);
    }

    println!("done");
}
