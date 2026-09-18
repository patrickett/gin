use flask::{FlaskConfigHandle, GinHome, RegistryClient};

pub fn registry(update: bool, offline: bool) -> Result<(), String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let handle = FlaskConfigHandle::load(&cwd).map_err(|e| e.to_string())?;
    let mut config = handle.read().config.clone();
    let client = RegistryClient::load(GinHome::discover().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();
    if update {
        let resolution = client.resolve(&config, now).map_err(|e| e.to_string())?;
        config.resolution = Some(resolution.clone());
        client
            .fetch(&config, now, false)
            .map_err(|e| e.to_string())?;
        handle
            .update_resolution(resolution)
            .map_err(|e| e.to_string())?;
        println!("Updated and verified the manifest resolution");
    } else {
        if offline {
            client
                .verify_cached(&config, now)
                .map_err(|e| e.to_string())?;
        } else {
            client
                .fetch(&config, now, true)
                .map_err(|e| e.to_string())?;
        }
        println!("Verified dependencies are available in the user cache");
    }
    Ok(())
}
