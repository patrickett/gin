//! MLIR code generation infrastructure.

mod lower;
mod mlir_ext;
pub mod emit;

pub use lower::*;
pub use mlir_ext::*;
pub use emit::*;

/// Prelude for codegen implementations - includes MLIR types
pub mod prelude {
    pub use crate::{
        CodegenContext, Lower, RuntimeSymbolTable, TypeInfo,
        // Extension traits
        ArithOps, AttributeExt, BlockExt, ContextExt, FPredicates, OperationBuilderExt, Predicates,
    };
    pub use ::ast::*;
    pub use chumsky::{input::ValueInput, prelude::*};
    pub use melior::{
        Context,
        dialect::{arith as arith_dialect, llvm::r#type, scf as scf_dialect},
        ir::{
            Attribute, Block, BlockLike, BlockRef, Identifier, Location, Module, Operation, Region,
            RegionLike, Type, Value, ValueLike,
            attribute::{
                DenseI32ArrayAttribute, DenseI64ArrayAttribute, FlatSymbolRefAttribute,
                FloatAttribute, IntegerAttribute, StringAttribute, TypeAttribute,
            },
            operation::OperationBuilder,
            r#type::{id::*, IntegerType},
        },
    };
}
