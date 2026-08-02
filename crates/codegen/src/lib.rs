//! MLIR code generation infrastructure.

pub mod emit;
mod lower;
mod mlir_ext;

pub use emit::{NativeCompiler, Profile};
pub use lower::CodegenContext;
pub use mlir_ext::{AttributeExt, BlockExt, ContextExt, OperationBuilderExt, Predicates};

use internment::Intern;
use melior::ir::Value;
use std::collections::HashMap;
use typecheck::ty::Ty;

/// A variable binding in the scoped symbol table.
#[derive(Debug, Clone)]
pub struct Slot<'c> {
    /// The MLIR value (SSA value or alloca pointer).
    pub value: Value<'c, 'c>,
    /// The resolved type of this variable.
    pub ty: Ty,
    /// True if this variable has a stack slot (declared `:type` without a value, or
    /// assigned after its first bind). Rebinding stores through the existing pointer
    /// instead of creating a new SSA value.
    pub is_slot: bool,
}

/// Scoped symbol table for MLIR values during codegen.
/// This replaces RuntimeSymbolTable to avoid expensive HashMap cloning on every scope entry.
/// Instead, scopes are pushed onto a stack and popped when exiting, providing O(1) scope management.
///
/// Each entry stores the MLIR value, its resolved type, and whether it owns a stack slot —
/// eliminating the need for separate `var_types` and `mutable_slots` tracking.
#[derive(Clone)]
pub struct ScopedSymbolTable<'c> {
    /// Stack of scopes, where each scope maps interned variable names to Slot info.
    scopes: Vec<HashMap<Intern<String>, Slot<'c>>>,
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
    pub fn insert(&mut self, name: Intern<String>, value: Value<'c, 'c>, ty: Ty, is_slot: bool) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, Slot { value, ty, is_slot });
        }
    }

    /// Look up a variable by name, searching from innermost to outermost scope.
    /// Returns the Slot from the first scope that contains the name.
    pub fn get(&self, name: &Intern<String>) -> Option<&Slot<'c>> {
        for scope in self.scopes.iter().rev() {
            if let Some(slot) = scope.get(name) {
                return Some(slot);
            }
        }
        None
    }

    /// Look up a variable and return just its MLIR value.
    pub fn get_value(&self, name: &Intern<String>) -> Option<Value<'c, 'c>> {
        self.get(name).map(|s| s.value)
    }

    /// Check if a variable exists and is a stack slot.
    pub fn is_slot(&self, name: &Intern<String>) -> bool {
        self.get(name).is_some_and(|s| s.is_slot)
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
        AttributeExt,
        BlockExt,
        CodegenContext,
        ContextExt,
        OperationBuilderExt,
        Predicates,
        ScopedSymbolTable,
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
