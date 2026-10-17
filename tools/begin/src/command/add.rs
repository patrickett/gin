use clap::Args;
use flask::{FlaskConfig, FlaskConfigHandle, MinimumRequirement};

#[derive(Args, Debug)]
pub struct AddArgs {
    #[arg(value_name = "PACKAGE")]
    package: String,
    #[arg(long)]
    alias: Option<String>,
}

pub fn begin_add(_config: FlaskConfig, args: &AddArgs) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let handle = FlaskConfigHandle::load(&cwd).map_err(|e| e.to_string())?;
    if args.package.starts_with("./") || args.package.starts_with("../") {
        let path = handle.source_dir().join(&args.package);
        let (dependency, root) = FlaskConfig::discover(&path)?
            .ok_or_else(|| "local dependency has no manifest".to_owned())?;
        if root.canonicalize().map_err(|e| e.to_string())?
            != path.canonicalize().map_err(|e| e.to_string())?
        {
            return Err("local dependency must name a package root".into());
        }
        let alias = args.alias.as_deref().unwrap_or(dependency.name());
        handle
            .add_path(alias, &args.package)
            .map_err(|e| e.to_string())?;
    } else {
        let (name, _) = args
            .package
            .split_once('@')
            .ok_or_else(|| "expected package@version or a relative path".to_owned())?;
        let alias = args.alias.as_deref().unwrap_or(name);
        let requirement =
            MinimumRequirement::parse(alias, &args.package).map_err(|e| e.to_string())?;
        add_registry(&handle, alias, requirement)?;
    }
    Ok(())
}

fn add_registry(
    handle: &FlaskConfigHandle,
    alias: &str,
    requirement: MinimumRequirement,
) -> Result<(), String> {
    let config = handle.read().config.clone();
    let workspace = config
        .with_workspace_requirements(&handle.source_dir())
        .map_err(|e| e.to_string())?;
    let client = super::registry_client()?;
    let now = super::now_seconds()?;
    let current = client.resolve(&workspace, now).map_err(|e| e.to_string())?;
    let next = config
        .with_minimum_floors(&current)
        .map_err(|e| e.to_string())?
        .with_minimum_requirement(alias, requirement)
        .map_err(|e| e.to_string())?;
    let effective = next
        .with_workspace_requirements(&handle.source_dir())
        .map_err(|e| e.to_string())?;
    client
        .fetch(&effective, now, false)
        .map_err(|e| e.to_string())?;
    handle
        .update_minimums(
            &next
                .registry_minimum_requirements()
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
}
