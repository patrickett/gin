use serde::Serialize;
use std::cell::RefCell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::Instant;

#[cfg(feature = "allocations")]
mod allocator;
#[cfg(feature = "allocations")]
pub use allocator::TrackingAllocator;

macro_rules! phases {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy, PartialEq, Eq)]
        #[repr(usize)]
        pub enum Phase { $($variant),+ }
        const PHASES: &[Phase] = &[$(Phase::$variant),+];
        impl Phase {
            pub fn name(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
        }
    };
}

phases! {
    Other => "other",
    Parse => "parse",
    Lex => "lex",
    Prepare => "prepare",
    FrontEnd => "front-end",
    AstClone => "ast-clone",
    PackagePass => "package-pass",
    Declarations => "declarations",
    Signatures => "signatures",
    Context => "context",
    EvalAst => "eval-ast",
    Traits => "traits",
    Interface => "interface",
    NativeEmission => "native-emission",
    MlirOptimization => "mlir-optimization",
    LlvmLowering => "llvm-lowering",
    ObjectEmission => "object-emission",
    Lowering => "lowering",
    Staging => "staging",
    References => "references",
    Effects => "effects",
    Finishing => "finishing",
    Refinements => "refinements",
    Representations => "representations",
    Terminals => "terminals",
    Consumption => "consumption",
    Rematerialize => "rematerialize",
    Flow => "flow",
    Constants => "constants",
    Specialization => "specialization",
    CompileTime => "compile-time",
    Discover => "discover-sources",
    Read => "read-sources",
    Diagnostics => "typecheck-diagnostics",
    Mlir => "mlir-lowering",
    ArtifactKey => "artifact-key",
    ObjectCache => "object-cache-lookup",
    Linking => "linking",
}

#[derive(Default)]
struct Counters {
    calls: AtomicU64,
    inclusive_ns: AtomicU64,
    self_ns: AtomicU64,
    allocations: AtomicU64,
    allocated_bytes: AtomicU64,
    frees: AtomicU64,
    freed_bytes: AtomicU64,
    reallocations: AtomicU64,
}

struct Recorder {
    phases: [Counters; PHASES.len()],
}

#[derive(Clone)]
pub struct Context {
    recorder: Arc<Recorder>,
    phase: Phase,
}

struct Active {
    context: Context,
    child_ns: u64,
}

thread_local! {
    static CURRENT: RefCell<Option<Active>> = const { RefCell::new(None) };
}
static CAPTURES: AtomicUsize = AtomicUsize::new(0);

pub struct Span {
    previous: Option<Active>,
    start: Option<Instant>,
    thread_bound: PhantomData<Rc<()>>,
}

pub fn span(phase: Phase) -> Span {
    let context = Context::current().map(|context| Context { phase, ..context });
    enter(context)
}

fn enter(context: Option<Context>) -> Span {
    let start = context.as_ref().map(|_| Instant::now());
    let previous = if let Some(context) = context {
        CURRENT.with(|slot| {
            slot.replace(Some(Active {
                context,
                child_ns: 0,
            }))
        })
    } else {
        None
    };
    Span {
        previous,
        start,
        thread_bound: PhantomData,
    }
}

impl Context {
    pub fn current() -> Option<Self> {
        if CAPTURES.load(Relaxed) == 0 {
            return None;
        }
        CURRENT.with(|slot| slot.borrow().as_ref().map(|active| active.context.clone()))
    }

    pub fn enter(&self) -> Span {
        let attached = CURRENT.with(|slot| {
            slot.borrow().as_ref().is_some_and(|active| {
                Arc::ptr_eq(&active.context.recorder, &self.recorder)
                    && active.context.phase == self.phase
            })
        });
        enter((!attached).then(|| self.clone()))
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        let Some(start) = self.start else {
            return;
        };
        let elapsed = start.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        let current = CURRENT
            .with(|slot| slot.replace(self.previous.take()))
            .unwrap();
        let counters = &current.context.recorder.phases[current.context.phase as usize];
        counters.calls.fetch_add(1, Relaxed);
        counters.inclusive_ns.fetch_add(elapsed, Relaxed);
        counters
            .self_ns
            .fetch_add(elapsed.saturating_sub(current.child_ns), Relaxed);
        CURRENT.with(|slot| {
            if let Some(parent) = slot.borrow_mut().as_mut() {
                parent.child_ns = parent.child_ns.saturating_add(elapsed);
            }
        });
    }
}

#[derive(Serialize)]
pub struct PhaseReport {
    pub name: &'static str,
    pub calls: u64,
    pub inclusive_seconds: f64,
    pub self_seconds: f64,
    pub allocations: u64,
    pub allocated_bytes: u64,
    pub frees: u64,
    pub freed_bytes: u64,
    pub reallocations: u64,
}

#[derive(Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub wall_seconds: f64,
    pub allocation_tracking: bool,
    pub allocation_scope: &'static str,
    pub timing_scope: &'static str,
    pub rust_live_bytes_before: Option<u64>,
    pub rust_live_bytes_after: Option<u64>,
    pub rust_process_lifetime_peak_bytes: Option<u64>,
    pub phases: Vec<PhaseReport>,
}

struct Capture;
impl Drop for Capture {
    fn drop(&mut self) {
        CAPTURES.fetch_sub(1, Relaxed);
    }
}

pub fn capture<T>(action: impl FnOnce() -> T) -> (T, Report) {
    let recorder = Arc::new(Recorder {
        phases: std::array::from_fn(|_| Counters::default()),
    });
    CAPTURES.fetch_add(1, Relaxed);
    let active = Capture;
    let before = memory();
    let start = Instant::now();
    let root = enter(Some(Context {
        recorder: recorder.clone(),
        phase: Phase::Other,
    }));
    let result = action();
    drop(root);
    let wall_seconds = start.elapsed().as_secs_f64();
    let after = memory();
    drop(active);
    let phases = PHASES
        .iter()
        .filter_map(|phase| {
            let counters = &recorder.phases[*phase as usize];
            let calls = counters.calls.load(Relaxed);
            (calls > 0).then(|| PhaseReport {
                name: phase.name(),
                calls,
                inclusive_seconds: counters.inclusive_ns.load(Relaxed) as f64 / 1e9,
                self_seconds: counters.self_ns.load(Relaxed) as f64 / 1e9,
                allocations: counters.allocations.load(Relaxed),
                allocated_bytes: counters.allocated_bytes.load(Relaxed),
                frees: counters.frees.load(Relaxed),
                freed_bytes: counters.freed_bytes.load(Relaxed),
                reallocations: counters.reallocations.load(Relaxed),
            })
        })
        .collect();
    (
        result,
        Report {
            schema_version: 1,
            wall_seconds,
            allocation_tracking: after.is_some(),
            allocation_scope: "Rust requested bytes; realloc counts old freed and new allocated; frees belong to the freeing phase; excludes native heaps and subprocesses",
            timing_scope: "inclusive spans overlap; self time excludes same-thread children, includes waits; worker times can overlap caller and each other",
            rust_live_bytes_before: before.map(|value| value.0),
            rust_live_bytes_after: after.map(|value| value.0),
            rust_process_lifetime_peak_bytes: after.map(|value| value.1),
            phases,
        },
    )
}

fn memory() -> Option<(u64, u64)> {
    #[cfg(feature = "allocations")]
    {
        allocator::memory()
    }
    #[cfg(not(feature = "allocations"))]
    {
        None
    }
}

#[cfg(test)]
mod tests;
