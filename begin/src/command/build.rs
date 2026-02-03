use flask::FlaskConfig;
use ginc::{Args, GinCompiler};
use std::path::PathBuf;

// TODO: compiler performance, show time spend on io, and the number of syscalls

/// `begin (b)uild` will build
pub fn begin_build(_config: FlaskConfig, input: Option<PathBuf>) {
    // check if we have a `main.gin` if so we build binary
    // otherwise we build a gin library
    let path = input.or_else(|| std::env::current_dir().ok());

    let Some(path) = path else {
        todo!("fancy error message for bad path")
    };

    if !path.exists() {
        todo!("fancy error message for path 404")
    }

    let mut args = Args {
        input: path,
        ..Default::default()
    };

    GinCompiler::compile(&mut args)
}
