//! MLIR code generation infrastructure.

mod lower;
mod mlir_ext;

pub use lower::*;
pub use mlir_ext::*;

/// Prelude for codegen implementations - includes MLIR types
pub mod prelude {
    pub use crate::prelude::*;
    pub use crate::codegen::{
        CodegenContext, Lower, RuntimeSymbolTable, TypeInfo,
        // Extension traits
        ArithOps, AttributeExt, BlockExt, ContextExt, OperationBuilderExt, Predicates,
    };
    pub use melior::{
        Context,
        dialect::llvm::r#type,
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
