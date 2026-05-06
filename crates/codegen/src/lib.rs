//! MLIR code generation infrastructure.

pub mod emit;
mod lower;
mod mlir_ext;

pub use emit::*;
pub use lower::*;
pub use mlir_ext::*;

use melior::ir::Value;
use std::collections::HashMap;

/// Scoped symbol table for MLIR values during codegen.
/// This replaces RuntimeSymbolTable to avoid expensive HashMap cloning on every scope entry.
/// Instead, scopes are pushed onto a stack and popped when exiting, providing O(1) scope management.
#[derive(Clone)]
pub struct ScopedSymbolTable<'c> {
    /// Stack of scopes, where each scope is a HashMap from variable name to MLIR Value.
    /// The last scope in the vec is the current (innermost) scope.
    scopes: Vec<HashMap<String, Value<'c, 'c>>>,
}

impl<'c> ScopedSymbolTable<'c> {
    /// Create a new scoped symbol table with an initial (global) scope.
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    /// Push a new scope onto the stack (e.g., when entering a block, loop, or function).
    /// This creates a new HashMap for the inner scope, allowing variables to shadow outer ones.
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Pop the current scope from the stack (e.g., when exiting a block, loop, or function).
    /// This removes the innermost scope and all its variables.
    ///
    /// # Panics
    /// Panics if there's only one scope (the global scope cannot be popped).
    pub fn pop_scope(&mut self) {
        assert!(
            self.scopes.len() > 1,
            "Cannot pop the global scope from ScopedSymbolTable"
        );
        self.scopes.pop();
    }

    /// Insert a variable binding in the current (innermost) scope.
    pub fn insert(&mut self, name: String, value: Value<'c, 'c>) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, value);
        }
    }

    /// Look up a variable by name, searching from innermost to outermost scope.
    /// Returns the value from the first scope that contains the name.
    pub fn get(&self, name: &str) -> Option<Value<'c, 'c>> {
        // Search scopes from innermost to outermost
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Some(*value);
            }
        }
        None
    }

    /// Check if a variable exists in any scope.
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// Get the number of active scopes (always at least 1).
    pub fn depth(&self) -> usize {
        self.scopes.len()
    }
}

impl<'c> Default for ScopedSymbolTable<'c> {
    fn default() -> Self {
        Self::new()
    }
}

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
        ScopedSymbolTable,
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
