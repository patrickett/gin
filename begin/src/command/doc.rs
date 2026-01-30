use clap::Subcommand;
use flask::FlaskConfig;

#[derive(Subcommand, Debug, Default)]
pub enum DocCommand {
    /// Opens the docs in a browser after the building
    #[command(alias = "o")]
    #[default]
    Open,
}

/// `begin doc` will build docs
pub fn begin_doc(_config: FlaskConfig) {
    #[cfg(debug_assertions)]
    println!("info: generating docs...");

    // Ok((warnings, ()))
}
