use super::*;

fn generation(id: u64) -> GenerationId {
    GenerationId(id)
}

fn generation_exports(logical_symbol: &str, id: u64) -> Vec<NativeExport> {
    vec![NativeExport {
        logical_symbol: logical_symbol.to_string(),
        generation_symbol: runtime_symbol_name(logical_symbol, GenerationId(id)),
    }]
}

#[test]
fn install_generation_replaces_symbols_and_retains_only_live_if_inactive() {
    let mut runtime = LiveRuntime::default();
    runtime.install_generation(NativeGeneration {
        id: generation(1),
        engine: None,
        exports: generation_exports("value", 1),
    });
    runtime.install_generation(NativeGeneration {
        id: generation(2),
        engine: None,
        exports: generation_exports("value", 2),
    });

    assert_eq!(runtime.generations().len(), 2);
    runtime.retire_unused_generations();
    assert_eq!(runtime.generations().len(), 1);
    assert_eq!(runtime.generation_for_symbol("value"), Some(generation(2)));
}

#[test]
fn active_calls_pin_generations_until_call_finishes() {
    let mut runtime = LiveRuntime::default();
    runtime.install_generation(NativeGeneration {
        id: generation(1),
        engine: None,
        exports: generation_exports("value", 1),
    });
    let symbols = runtime.symbols().clone();
    let active_call = runtime.track_active_generations_for_symbols(&symbols);
    runtime.install_generation(NativeGeneration {
        id: generation(2),
        engine: None,
        exports: generation_exports("value", 2),
    });
    assert_eq!(runtime.generations().len(), 2);
    assert_eq!(runtime.generation_for_symbol("value"), Some(generation(2)));
    runtime.finish_active_generations(active_call);
    assert_eq!(runtime.generations().len(), 1);
    assert_eq!(runtime.generation_for_symbol("value"), Some(generation(2)));
}

#[test]
fn clear_removes_active_generation_tracking() {
    let mut runtime = LiveRuntime::default();
    runtime.install_generation(NativeGeneration {
        id: generation(1),
        engine: None,
        exports: generation_exports("value", 1),
    });
    let symbols = runtime.symbols().clone();
    let _active_call = runtime.track_active_generations_for_symbols(&symbols);
    runtime.clear();
    assert!(runtime.generations().is_empty());
    assert!(runtime.symbols().is_empty());
    assert!(runtime.values().is_empty());
}
