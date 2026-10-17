use flask::FlaskConfigHandle;

pub fn registry(update: bool, offline: bool, target: Option<&str>) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let handle = FlaskConfigHandle::load(&cwd).map_err(|e| e.to_string())?;
    let mut config = handle.read().config.clone();
    let workspace = config
        .with_workspace_requirements(&handle.source_dir())
        .map_err(|e| e.to_string())?;
    if workspace.minimum_version_selection
        && workspace
            .registry_minimum_requirements()
            .map_err(|e| e.to_string())?
            .is_empty()
    {
        if target.is_some() {
            return Err("update target is not a dependency; use begin add".into());
        }
        println!("No registry dependencies");
        return Ok(());
    }
    let client = super::registry_client()?;
    let now = super::now_seconds()?;
    if update {
        let resolution = match target {
            Some(target) => {
                let (name, _) = target
                    .split_once('@')
                    .ok_or_else(|| "expected package@version".to_owned())?;
                let requirement =
                    flask::MinimumRequirement::parse(name, target).map_err(|e| e.to_string())?;
                client.update_requirement(&workspace, &requirement, now)
            }
            None => client.update(&workspace, now),
        }
        .map_err(|e| e.to_string())?;
        config = config
            .with_minimum_floors(&resolution)
            .map_err(|e| e.to_string())?;
        let next = config
            .with_workspace_requirements(&handle.source_dir())
            .map_err(|e| e.to_string())?;
        client.fetch(&next, now, false).map_err(|e| e.to_string())?;
        handle
            .update_minimums(
                &config
                    .registry_minimum_requirements()
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        println!("Updated minimum dependency versions");
    } else {
        if offline {
            client
                .verify_cached(&workspace, now)
                .map_err(|e| e.to_string())?;
        } else {
            client
                .fetch(&workspace, now, true)
                .map_err(|e| e.to_string())?;
        }
        println!("Verified dependencies are available in the user cache");
    }
    Ok(())
}
