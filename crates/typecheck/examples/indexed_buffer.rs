//! Check the real three-module witness; do not execute Gin or supply effects.
use typecheck::{CheckedSourceModule, Limits, check_module};

fn verify(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, String> {
    let source = parser::parse_strict(text)
        .map_err(|errors| format!("{name}: source diagnostics: {errors:?}"))?;
    check_module(name, &source, dependencies, Limits::default())
        .map_err(|error| format!("{name}: checking failed: {error:?}"))
}

fn main() -> Result<(), String> {
    if std::env::args_os().len() != 1 {
        return Err("usage: indexed_buffer".into());
    }
    let slots = verify("example.slots", include_str!("runtime-slots.gin"), &[])?;
    let buffer = verify(
        "example.buffers",
        include_str!("indexed-buffer.gin"),
        &[&slots],
    )?;
    let client = verify(
        "example.client",
        include_str!("indexed-buffer-client.gin"),
        &[&buffer],
    )?;
    for module in [&slots, &buffer, &client] {
        println!(
            "{}: {} checked functions",
            module.name(),
            module.checked().functions.len()
        );
    }
    Ok(())
}
