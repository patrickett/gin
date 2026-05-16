mod native;
mod toolchain;

/// Build profile for optimization levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    #[default]
    Debug,
    Release,
}

impl Profile {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

pub use native::{
    build_module_text, build_module_text_from_typed, compile_to_object,
    compile_to_object_from_typed, link_executable, native_from_mlir, native_from_module,
};
