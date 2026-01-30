use flask::FlaskConfig;

/// `begin doc` will build docs
pub fn begin_doc(_config: FlaskConfig) {
    #[cfg(debug_assertions)]
    println!("info: generating docs...");

    // Ok((warnings, ()))
}
