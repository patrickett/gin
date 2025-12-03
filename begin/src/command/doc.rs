use crate::flask::FlaskConfig;
use ginc::GincResult;

/// `begin doc` will build docs
pub fn begin_doc(_config: FlaskConfig) -> GincResult<()> {
    #[cfg(debug_assertions)]
    println!("info: generating docs...");

    let warnings = Vec::new();
    Ok((warnings, ()))
}
