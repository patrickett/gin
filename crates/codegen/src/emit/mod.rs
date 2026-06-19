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

impl std::fmt::Display for Profile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Profile {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "debug" => Ok(Self::Debug),
            "release" => Ok(Self::Release),
            _ => Err(format!(
                "unknown build profile '{s}', expected 'debug' or 'release'"
            )),
        }
    }
}

pub use native::NativeCompiler;
