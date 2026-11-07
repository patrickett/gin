use std::error::Error;

use typecheck::{Limits, check_file};

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let path = arguments
        .next()
        .ok_or("usage: source_check <single-file.gin>")?;
    if arguments.next().is_some() {
        return Err("usage: source_check <single-file.gin>".into());
    }
    let text = std::fs::read_to_string(path)?;
    let source = parser::parse_strict(&text)
        .map_err(|diagnostics| format!("source parse failed: {diagnostics:?}"))?;
    let checked = check_file(&source, Limits::default())
        .map_err(|error| format!("source check failed: {error:?}"))?;
    println!("source: checked {} functions", checked.functions.len());
    for (id, aggregate) in checked.aggregates.iter().enumerate() {
        println!("  schema {id}: {aggregate:?}");
    }
    for (id, instance) in checked.instances.iter().enumerate() {
        println!("  instance {id}: {instance:?}");
    }
    for function in checked.functions {
        println!("  {function:?}");
    }
    Ok(())
}
