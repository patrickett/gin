use std::error::Error;

use typecheck::{Limits, check_module};

fn main() -> Result<(), Box<dyn Error>> {
    let usage = "usage: source_modules <library-name> <library-file> <client-name> <client-file>";
    let mut arguments = std::env::args_os().skip(1);
    let library_name = arguments.next().ok_or(usage)?;
    let library_path = arguments.next().ok_or(usage)?;
    let client_name = arguments.next().ok_or(usage)?;
    let client_path = arguments.next().ok_or(usage)?;
    if arguments.next().is_some() {
        return Err(usage.into());
    }
    let library_name = library_name.to_str().ok_or("library name must be UTF-8")?;
    let client_name = client_name.to_str().ok_or("client name must be UTF-8")?;
    let library_text = std::fs::read_to_string(library_path)?;
    let library_source = parser::parse_strict(&library_text)
        .map_err(|errors| format!("library parse failed: {errors:?}"))?;
    let library = check_module(library_name, &library_source, &[], Limits::default())
        .map_err(|error| format!("library check failed: {error:?}"))?;
    let client_text = std::fs::read_to_string(client_path)?;
    let client_source = parser::parse_strict(&client_text)
        .map_err(|errors| format!("client parse failed: {errors:?}"))?;
    let client = check_module(client_name, &client_source, &[&library], Limits::default())
        .map_err(|error| format!("client check failed: {error:?}"))?;
    println!(
        "library {}: {} functions",
        library.name(),
        library.checked().functions.len()
    );
    println!(
        "client {}: {} functions",
        client.name(),
        client.checked().functions.len()
    );
    for function in &client.checked().functions {
        println!("  {function:?}");
    }
    Ok(())
}
