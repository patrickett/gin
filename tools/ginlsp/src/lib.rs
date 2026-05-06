// ginlsp library — shared analysis functions used by both ginlsp (LSP) and
// ginmcp (MCP). This is the dogfooding layer: ginmcp depends on ginlsp
// instead of calling the low‑level crates directly.

pub use ast;
pub use diagnostic;
pub use ginfmt;
pub use parser;
pub use resolve;
pub use typeck;

pub mod analysis;
pub mod json;
