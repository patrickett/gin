/// Custom infoing macro that formats messages consistently.
///
/// # Usage
///
/// The `info!` macro can be used throughout the codebase to emit formatted
/// infoing messages. It works similarly to Rust's standard macros like `println!`
/// but automatically prefixes all output with "info: " for consistency.
#[macro_export]
macro_rules! info {
    ($fmt:literal $(, $arg:expr)* $(,)?) => {
        eprintln!(concat!("", $fmt), $($arg),*)
    };
}
