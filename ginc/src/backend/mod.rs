//! Backend is responsible for transforming intermediate representations of code into machine code or assembly language.

pub mod codegen;
pub mod compile;

pub mod prelude {
    pub use crate::backend::codegen::*;
    pub use crate::frontend::prelude::*;
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
            r#type::{id::*, *},
        },
    };
}
