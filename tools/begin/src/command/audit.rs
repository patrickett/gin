use flask::{FlaskConfig, GinHome, RegistryClient};

pub fn begin_audit(config: FlaskConfig) {
    let result = (|| -> Result<(), String> {
        let client = RegistryClient::load(GinHome::discover().map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();
        client.audit(&config, now).map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => println!(
            "Every locked registry dependency is eligible under current signed security metadata"
        ),
        Err(error) => {
            eprintln!("begin audit: {error}");
            std::process::exit(1);
        }
    }
}
