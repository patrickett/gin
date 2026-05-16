// ginlsp library — shared analysis functions used by ginlsp (LSP).
// This is the dogfooding layer that provides analysis primitives.

pub use ast;
pub use diagnostic;
pub use ginfmt;
pub use parser;
pub use resolve;

pub mod analysis;
pub mod json;
