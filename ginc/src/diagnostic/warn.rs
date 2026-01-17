use crate::diagnostic::Printable;

pub type GincWarnings = Vec<GincWarn>;

#[derive(Debug)]
pub enum GincWarn {
    UnusedImport(std::path::PathBuf),
    UnusedDefinition(String),
    UnusedTag(String),
}

/// Custom warning macro that formats messages consistently.
///
/// # Usage
///
/// The `warn!` macro can be used throughout the codebase to emit formatted
/// warning messages. It works similarly to Rust's standard macros like `println!`
/// but automatically prefixes all output with "warn: " for consistency.
#[macro_export]
macro_rules! warn {
    ($fmt:literal $(, $arg:expr)* $(,)?) => {
        eprintln!(concat!("warn: ", $fmt), $($arg),*)
    };
}

// Implement Printable for GincWarn
impl Printable for GincWarn {
    fn print(&self) {
        match self {
            GincWarn::UnusedImport(path) => {
                warn!("unused import '{}'", path.display());
            }
            GincWarn::UnusedDefinition(name) => {
                warn!("unused definition '{}'", name);
            }
            GincWarn::UnusedTag(tag) => {
                warn!("unused tag '{}'", tag);
            }
        }
    }
}

impl Printable for GincWarnings {
    fn print(&self) {
        for warning in self {
            warning.print();
        }
    }
}
