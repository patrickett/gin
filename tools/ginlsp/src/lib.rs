//! Gin diagnostics, source queries and formatting over the Language Server Protocol.
//!
//! Checking is `ginterpreter::check` (the shared compiler diagnostic path), packages
//! are `package`, hover is `ginterpreter::inspect`, and syntax queries are
//! `ast`. This crate only maps their answers onto the Language Server Protocol.

pub mod analysis;
pub mod diagnostics;
pub mod hover;
pub mod navigate;
pub mod position;
pub mod server;
pub mod symbols;
pub mod transport;
