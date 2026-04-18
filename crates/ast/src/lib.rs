//! AST type definitions for the Gin compiler.

pub mod expr;
pub use expr::FormatPart;
pub use expr::*;

mod file_ast;
pub use file_ast::*;

mod module_qualify;
pub use module_qualify::qualify_module_defs;

mod parameter;
pub use parameter::*;

mod path;
pub use path::*;

mod variant;
pub use variant::*;

mod doc_comment;
pub use doc_comment::*;

mod declare;
pub use declare::*;

mod pattern;
pub use pattern::*;

pub mod span;
pub use span::*;

mod impl_block;
pub use impl_block::*;

pub mod signature;
pub use signature::*;

pub mod prelude {
    pub use crate::declare::*;
    pub use crate::doc_comment::*;
    pub use crate::expr::*;
    pub use crate::file_ast::*;
    pub use crate::impl_block::*;
    pub use crate::parameter::*;
    pub use crate::path::*;
    pub use crate::pattern::*;
    pub use crate::span::*;

    pub use crate::variant::*;

    pub use internment::Intern;
}
