//! MLIR code generation infrastructure.

pub mod emit;
mod lower;
mod mlir_ext;

pub use emit::*;
pub use lower::*;
pub use mlir_ext::*;

/// Prelude for codegen implementations - includes MLIR types
pub mod prelude {
    pub use crate::{
        // Extension traits
        ArithOps,
        AttributeExt,
        BlockExt,
        CodegenContext,
        ContextExt,
        FPredicates,
        Lower,
        OperationBuilderExt,
        Predicates,
        RuntimeSymbolTable,
        TypeInfo,
    };
    pub use ::ast::*;
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
            r#type::{IntegerType, id::*},
        },
    };
}
