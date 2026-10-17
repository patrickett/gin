use flask::FlaskConfig;

pub fn begin_audit(config: FlaskConfig) {
    let result = (|| -> Result<(), String> {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        let (_, root) =
            FlaskConfig::discover(&cwd)?.ok_or_else(|| "no package manifest".to_owned())?;
        let config = config
            .with_workspace_requirements(&root)
            .map_err(|e| e.to_string())?;
        if config.minimum_version_selection
            && config
                .registry_minimum_requirements()
                .map_err(|e| e.to_string())?
                .is_empty()
        {
            return Ok(());
        }
        let client = super::registry_client()?;
        let now = super::now_seconds()?;
        client.audit(&config, now).map_err(|e| e.to_string())
    })();
    match result {
        Ok(()) => println!(
            "Every selected registry dependency is eligible under current signed security metadata"
        ),
        Err(error) => {
            eprintln!("begin audit: {error}");
            std::process::exit(1);
        }
    }
}
