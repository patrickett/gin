mod flaw;
mod info;
mod warn;

pub use flaw::*;
// pub use info::*;
pub use warn::*;

// Trait that will print the underlying type to the console
pub trait Printable {
    /// Print to the console
    fn print(&self);
}
