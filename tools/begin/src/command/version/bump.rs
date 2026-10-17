use flask::{FlaskConfigHandle, VersionBump};
use ginc::cli::{Args, Emit};
use ginc::compile::{CompileResult, GinCompiler};
use interface::PublicInterface;
use semver::Version;
use std::{fs, path::Path};

pub fn bump(against: Option<&Path>, check: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let handle = FlaskConfigHandle::load(&cwd).map_err(|e| e.to_string())?;
    let config = handle.read().config.clone();
    let bytes = if let Some(path) = against {
        fs::read(path).map_err(|e| e.to_string())?
    } else {
        let client = crate::command::registry_client()?;
        let now = crate::command::now_seconds()?;
        client
            .interface(
                &config.name,
                &Version::parse(&config.version).map_err(|e| e.to_string())?,
                now,
            )
            .map_err(|e| e.to_string())?
    };
    let old = PublicInterface::decode(&bytes).map_err(|e| e.to_string())?;
    if old.subject.package != config.name {
        return Err("baseline belongs to a different package".into());
    }
    let previous = Version::parse(&old.subject.version).map_err(|e| e.to_string())?;
    let current = Version::parse(&config.version).map_err(|e| e.to_string())?;
    let mut args = Args {
        input: handle.source_dir(),
        emit: Emit::Interface,
        ..Default::default()
    };
    let output = match GinCompiler::compile(&mut args) {
        CompileResult::Success { output: Some(path) } => path,
        CompileResult::Failed(error) => return Err(error.to_string()),
        _ => return Err("interface compilation must succeed without type errors".into()),
    };
    let new = PublicInterface::decode(&fs::read(output).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let bump = VersionBump::compare(&old, &new).map_err(|e| e.to_string())?;
    let next = bump.next(&previous).map_err(|e| e.to_string())?;
    if current != previous && current != next {
        return Err(format!(
            "manifest version {current} is neither baseline {previous} nor required version {next}"
        ));
    }
    println!("{}: {previous} -> {next} ({bump:?})", config.name);
    for old_decl in &old.declarations {
        match new
            .declarations
            .iter()
            .find(|decl| decl.reference == old_decl.reference)
        {
            None => println!("removed {:?}", old_decl.reference),
            Some(decl) if decl.fingerprint != old_decl.fingerprint => {
                println!("changed {:?}", decl.reference)
            }
            _ => {}
        }
    }
    for decl in &new.declarations {
        if !old
            .declarations
            .iter()
            .any(|old| old.reference == decl.reference)
        {
            println!("added {:?}", decl.reference);
        }
    }
    if !check && current != next {
        handle
            .update_version(&config.version, &next.to_string())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
