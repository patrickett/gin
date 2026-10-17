use super::*;

#[cfg(feature = "allocations")]
#[global_allocator]
static ALLOCATOR: TrackingAllocator<std::alloc::System> = TrackingAllocator(std::alloc::System);

#[test]
fn nested_time_is_not_double_counted() {
    let (_, report) = capture(|| {
        let _parent = span(Phase::FrontEnd);
        let _child = span(Phase::Lex);
        std::hint::black_box(43);
    });
    let sum: f64 = report.phases.iter().map(|phase| phase.self_seconds).sum();
    assert!(sum <= report.wall_seconds);
    let root = report
        .phases
        .iter()
        .find(|phase| phase.name == "other")
        .unwrap();
    assert!((root.inclusive_seconds - sum).abs() < 1e-12);
    assert_eq!(
        report
            .phases
            .iter()
            .find(|p| p.name == "lex")
            .unwrap()
            .calls,
        1
    );
    assert!(Context::current().is_none());
}

#[test]
fn worker_context_and_unwinding_restore_the_caller() {
    let (_, report) = capture(|| {
        let _parent = span(Phase::Lowering);
        let context = Context::current().unwrap();
        std::thread::spawn(move || {
            let _attached = context.enter();
            let _child = span(Phase::References);
        })
        .join()
        .unwrap();
        let _ = std::panic::catch_unwind(|| {
            let _child = span(Phase::Lex);
            panic!("test unwind");
        });
        assert_eq!(Context::current().unwrap().phase.name(), "lowering");
    });
    assert!(report.phases.iter().any(|p| p.name == "references"));
    assert!(Context::current().is_none());
}

#[test]
fn capture_unwind_and_nested_capture_are_isolated() {
    let _ = std::panic::catch_unwind(|| capture(|| panic!("test capture unwind")));
    assert!(Context::current().is_none());
    let (_, outer) = capture(|| {
        let (_, inner) = capture(|| {
            let _span = span(Phase::Lex);
        });
        assert!(inner.phases.iter().any(|p| p.name == "lex"));
    });
    assert!(!outer.phases.iter().any(|p| p.name == "lex"));
}

#[cfg(feature = "allocations")]
#[test]
fn allocation_growth_and_cross_phase_free_are_attributed_once() {
    let (_, report) = capture(|| {
        let mut bytes = {
            let _span = span(Phase::Lex);
            std::hint::black_box(vec![0u8; 1024])
        };
        {
            let _span = span(Phase::Parse);
            bytes.reserve_exact(2048);
            std::hint::black_box(&bytes);
            drop(bytes);
        }
    });
    let lex = report.phases.iter().find(|p| p.name == "lex").unwrap();
    let parse = report.phases.iter().find(|p| p.name == "parse").unwrap();
    assert!(report.allocation_tracking);
    assert_eq!(lex.allocated_bytes, 1024);
    assert_eq!(lex.freed_bytes, 0);
    assert_eq!(parse.reallocations, 1);
    assert_eq!(
        lex.allocated_bytes + parse.allocated_bytes,
        parse.freed_bytes
    );
}

#[cfg(feature = "allocations")]
#[test]
fn worker_allocations_follow_the_attached_phase() {
    let (_, report) = capture(|| {
        let context = Context::current().unwrap();
        std::thread::spawn(move || {
            let _attached = context.enter();
            let _phase = span(Phase::Lex);
            let bytes = std::hint::black_box(vec![0u8; 512]);
            std::hint::black_box(&bytes);
        })
        .join()
        .unwrap();
    });
    let lex = report.phases.iter().find(|p| p.name == "lex").unwrap();
    assert_eq!(lex.allocated_bytes, 512);
    assert_eq!(lex.freed_bytes, 512);
}
