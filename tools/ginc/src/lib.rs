pub mod analysis;
mod cli;
pub mod cache;
mod compile;
pub mod database;
pub mod emit;
pub mod lsp;
pub mod parse;
pub mod source;

// Re-export external crates as modules for test compatibility
pub use ast;
pub use codegen;
pub use typeck;

pub use cli::*;
pub use compile::GinCompiler;
pub use ::ast::{DefMap, FileAst, Symbol, SymbolKind, SymbolTable, TagMap};
pub use lsp::completions::{
    CompletionCandidate, CompletionKind, SignatureInfo, completions_for_ast, fn_call_at,
    format_params, signature_for_fn,
};
pub use lsp::hover::{find_definition_span, find_references};
pub use database::{
    File,
    input_database::{Db, InputDatabase},
};
pub use diagnostic::{Category, Symptom, SymptomSource};
pub use diagnostic;
pub use internment::Intern;
pub use lexer;
pub use lexer::is_comment_at;
pub use source::{
    get_char_at_position, get_number_at_position, get_word_at_position, is_identifier_char,
    is_in_comment, position_to_byte_offset, word_at_byte_offset,
};
pub use typeck::{Ty, TyEnv};

pub const GIN_FILE_EXT: &str = "gin";

pub mod prelude {
    pub use ::ast::*;
    pub use codegen::{CodegenContext, Lower, RuntimeSymbolTable};
    pub use internment::Intern;
    pub use lexer::{MAX_INDENT_DEPTH, Token};
    pub use ::ast::parse::ParserError;
    pub use chumsky::{input::ValueInput, prelude::*};
}
