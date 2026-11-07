// SCRATCH measurement harness (checker-cost investigation). Not committed.
// usage: checker_cost <package dir> [sweep_steps] [only-module-substring]
use std::time::Instant;
use package::{GraphLimits, Package};
use typecheck::{check_module_files_diagnostic, CheckedSourceModule, Limits, SourceCheckError};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = std::path::PathBuf::from(&args[1]);
    let steps: u32 = args.get(2).map_or(0, |s| s.parse().unwrap());
    let only = args.get(3).cloned();
    let package = Package::open(&root).expect("open");
    let graph = package.load_graph(GraphLimits::default()).expect("graph");
    let base = Limits::default().work_max;
    let limits = |work: u32| Limits { work_max: work, definitions_max: 65536, ..Limits::default() };
    let full = limits(u32::MAX / 2);
    let mut capsules: Vec<CheckedSourceModule> = Vec::new();
    for (index, module) in graph.modules.iter().enumerate() {
        let deps: Vec<&CheckedSourceModule> = graph.imports[index].iter().map(|&d| &capsules[d]).collect();
        let ndeps_fns: usize = deps.iter().map(|d| d.checked().functions.len()).sum();
        let t = Instant::now();
        let checked = check_module_files_diagnostic(&module.name, &module.files, &deps, full);
        let secs = t.elapsed().as_secs_f64();
        let checked = match checked { Ok(c) => c, Err(e) => { println!("{} FAILED {:?}", module.name, e.error); return; } };
        let measure = only.as_deref().is_none_or(|o| module.name.contains(o));
        if measure {
            // bisection for minimal budget
            let (mut lo, mut hi) = (0u32, 1u32 << 31);
            while hi - lo > 1000 {
                let mid = lo + (hi - lo) / 2;
                match check_module_files_diagnostic(&module.name, &module.files, &deps, limits(mid)) {
                    Ok(_) => hi = mid,
                    Err(e) if e.error == SourceCheckError::Budget => lo = mid,
                    Err(e) => { println!("  non-budget error at {mid}: {:?}", e.error); lo = mid; }
                }
            }
            println!("{:<28} files={:<2} own_fns={:<5} dep_fns_total={:<6} work={:>11} ({:>6.1}x) time={:.3}s",
                module.name, module.files.len(), checked.checked().functions.len(), ndeps_fns, hi, hi as f64 / base as f64, secs);
            if steps > 0 {
                let mut last = String::new();
                let mut last_b = 0u32;
                for s in 1..=steps {
                    let b = (hi as u64 * s as u64 / steps as u64) as u32;
                    if let Err(e) = check_module_files_diagnostic(&module.name, &module.files, &deps, limits(b)) {
                        let label = match (e.file, e.span) {
                            (Some(f), Some(sp)) => {
                                let text = &module.files[f].text;
                                let r = sp.to_range();
                                let line = text[..r.start].matches('\n').count() + 1;
                                let head: String = text[r.start..].lines().next().unwrap_or("").chars().take(60).collect();
                                let cul = e.culprit.map(|c| { let cr = c.to_range(); let l = text[..cr.start].matches('\n').count()+1; format!(" @L{l}:{}", text[cr.start..cr.end.min(cr.start+40)].replace('\n'," ")) }).unwrap_or_default();
                                format!("{}:{} {}{}", module.paths[f].file_name().unwrap().to_string_lossy(), line, head.trim(), cul)
                            }
                            _ => format!("<no span> {:?}", e.detail),
                        };
                        if label != last { println!("   {:>10} {}", b, label); last = label; last_b = b; }
                    }
                }
                let _ = last_b;
            }
        }
        capsules.push(checked);
    }
}
