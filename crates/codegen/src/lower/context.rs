//! Codegen context carrying MLIR state, type info, and symptom collection.
//! This module provides:
//!
//! - [`CodegenContext`] — the central context object shared across all lowering
//! - Helper functions for computing line/column positions in source

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fmt,
};

use crate::ScopedSymbolTable;
use crate::mlir_ext::{BlockExt, ContextExt, OperationBuilderExt};
use ::span::{SpanId, SpanTable};
use ast::Pattern;
use ast::source::{LineStartsExt, SourceExt};
use diagnostic::Diagnostic;
use internment::Intern;
use melior::Context;
use melior::ir::Location;
use melior::ir::{BlockRef, Value, operation::OperationBuilder};
use typecheck::TypedFileAst;
use typecheck::VariantLookupResult;
use typecheck::compile_time_trait::CompileTimeTraitRegistry;
use typecheck::ty::Ty;

/// Collects codegen-level symptoms (errors, warnings) during lowering.
///
/// Write-only during lowering; drained at the end of codegen.
pub struct SymptomCollector {
    symptoms: RefCell<Vec<Diagnostic>>,
}

impl SymptomCollector {
    pub fn new() -> Self {
        Self {
            symptoms: RefCell::new(Vec::new()),
        }
    }

    pub fn emit(&self, diag: Diagnostic) {
        self.symptoms.borrow_mut().push(diag);
    }

    pub fn emit_internal(&self, message: impl Into<String>, span: SpanId, span_table: &SpanTable) {
        self.symptoms.borrow_mut().push(
            Diagnostic::new("codegen-internal", message.into())
                .with_help("an internal compiler error occurred")
                .at_span(span_table.get(span)),
        );
    }

    pub fn drain(&self) -> Vec<Diagnostic> {
        self.symptoms.borrow_mut().drain(..).collect()
    }
}

impl Default for SymptomCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SymptomCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SymptomCollector")
            .field("count", &self.symptoms.borrow().len())
            .finish()
    }
}

/// Manages string constant allocation for MLIR modules.
///
/// Each unique string gets a named global (`__string_N`). The registry
/// tracks allocation count and provides name-lookup for deduplication.
pub struct StringRegistry {
    literals: RefCell<Vec<String>>,
    symbols: RefCell<HashMap<String, String>>,
    counter: Cell<usize>,
}

impl StringRegistry {
    pub fn new() -> Self {
        Self {
            literals: RefCell::new(Vec::new()),
            symbols: RefCell::new(HashMap::new()),
            counter: Cell::new(0),
        }
    }

    /// Register a string and get its global symbol name.
    /// Returns the existing name if this string was already registered.
    pub fn register(&self, s: &str) -> String {
        {
            let symbols = self.symbols.borrow();
            if let Some(existing) = symbols.get(s) {
                return existing.clone();
            }
        }

        let counter = self.counter.get();
        let name = format!("__string_{}", counter);
        self.counter.set(counter + 1);

        self.symbols
            .borrow_mut()
            .insert(s.to_string(), name.clone());
        self.literals.borrow_mut().push(s.to_string());

        name
    }

    /// Consume the registry and return all registered string literals.
    pub fn into_literals(self) -> Vec<String> {
        self.literals.into_inner()
    }

    /// Iterate over (symbol_name, value) pairs for all registered strings.
    pub fn iter_symbols(&self) -> Vec<(String, String)> {
        self.symbols
            .borrow()
            .iter()
            .map(|(value, symbol)| (symbol.clone(), value.clone()))
            .collect()
    }
}

impl Default for StringRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for StringRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StringRegistry")
            .field("count", &self.counter.get())
            .finish()
    }
}

pub struct CodegenContext<'a, 'c> {
    pub mlir: &'c Context,
    /// Typed AST — carries tag_types, fn_return_types, variant_map, and ExprId-based traversal.
    pub typed_ast: Option<&'a TypedFileAst>,
    /// Compile-time trait registry (e.g. `Sized`, `Copy`) for querying type-level properties.
    pub trait_registry: Option<&'a CompileTimeTraitRegistry>,
    /// Intern<String>-keyed tag types (converted from TagId-keyed TypedFileAst on construction).
    pub tag_types: HashMap<Intern<String>, Ty>,
    /// Intern<String>-keyed fn return types (converted from DefId-keyed TypedFileAst on construction).
    pub fn_return_types: HashMap<Intern<String>, Ty>,
    pub strings: StringRegistry,
    pub symptoms: SymptomCollector,
    pub current_span: Cell<SpanId>,
    pub source_filename: String,
    pub source: &'a str,
    pub span_table: &'a SpanTable,
    pub line_starts: Vec<usize>,
}

#[allow(clippy::too_many_arguments)]
impl<'a, 'c> CodegenContext<'a, 'c> {
    pub fn new(
        mlir: &'c Context,
        typed_ast: Option<&'a TypedFileAst>,
        trait_registry: Option<&'a CompileTimeTraitRegistry>,
        source: &'a str,
        filename: &str,
        span_table: &'a SpanTable,
    ) -> Self {
        let (tag_types, fn_return_types) = match typed_ast {
            Some(typed) => {
                let tag_types = typed
                    .tag_types
                    .iter()
                    .map(|(id, ty)| (id.0, ty.clone()))
                    .collect();
                let fn_return_types = typed
                    .fn_return_types
                    .iter()
                    .map(|(id, ty)| (id.0, ty.clone()))
                    .collect();
                (tag_types, fn_return_types)
            }
            None => (HashMap::new(), HashMap::new()),
        };
        Self {
            mlir,
            typed_ast,
            trait_registry,
            tag_types,
            fn_return_types,
            strings: StringRegistry::new(),
            symptoms: SymptomCollector::new(),
            current_span: Cell::new(SpanId::INVALID),
            source_filename: filename.to_string(),
            source,
            span_table,
            line_starts: source.compute_line_starts(),
        }
    }

    pub fn lookup_variant(&self, name: Intern<String>) -> Option<VariantLookupResult<'_>> {
        self.typed_ast
            .and_then(|typed| typed.variant_map.get(&name))
            .and_then(|candidates| candidates.first())
            .map(|(union, idx, fields)| (*union, *idx, fields.as_slice()))
    }

    pub fn location(&self) -> Location<'c> {
        let id = self.current_span.get();
        if !id.is_valid() {
            return Location::new(self.mlir, &self.source_filename, 0, 0);
        }
        let span = self.span_table.get(id);
        let byte = (span.start()).min(self.source.len());
        let (line, col) = self.line_starts.byte_offset_to_line_col(byte);
        Location::new(self.mlir, &self.source_filename, line, col)
    }

    pub fn register_string(&self, s: &str) -> String {
        self.strings.register(s)
    }

    pub fn emit_symptom(&self, code: &str, message: String) {
        self.symptoms.emit(
            Diagnostic::new(code, message).at_span_id(self.current_span.get(), self.span_table),
        );
    }

    pub fn emit_internal(&self, message: impl Into<String>) {
        self.symptoms
            .emit_internal(message, self.current_span.get(), self.span_table);
    }

    pub fn drain_symptoms(&self) -> Vec<Diagnostic> {
        self.symptoms.drain()
    }

    /// Extend a small integer (i1 or i8) to i64 for comparison.
    /// - 2 variants  → zero-extend (arith.extui)
    /// - 3..256      → sign-extend (arith.extsi)
    /// - >256        → already i64, no-op
    pub fn emit_discriminant_extend(
        &self,
        block: &BlockRef<'c, 'c>,
        subject: Value<'c, 'c>,
        variant_count: usize,
    ) -> Value<'c, 'c> {
        let loc = self.location();
        if variant_count == 2 {
            let extend_op = match OperationBuilder::new("arith.extui", loc)
                .add_operands(&[subject])
                .add_results(&[self.mlir.i64()])
                .build()
            {
                Ok(op) => op,
                Err(e) => {
                    self.emit_internal(format!("extui build failed: {e}"));
                    return subject;
                }
            };
            block.append_op(extend_op)
        } else if variant_count <= 256 {
            let extend_op = match OperationBuilder::new("arith.extsi", loc)
                .add_operands(&[subject])
                .add_results(&[self.mlir.i64()])
                .build()
            {
                Ok(op) => op,
                Err(e) => {
                    self.emit_internal(format!("extsi build failed: {e}"));
                    return subject;
                }
            };
            block.append_op(extend_op)
        } else {
            subject
        }
    }

    /// Shared between `if_` pattern conditions and `when` pattern arms.
    /// Bind pattern payload fields from a subject value into the symbol table.
    pub fn bind_pattern_payload_fields(
        &self,
        block: &BlockRef<'c, 'c>,
        pattern: &Pattern,
        subject_val: Value<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) {
        if let Pattern::Generic { params, .. } = pattern {
            let variant_name = Intern::<String>::from_ref(pattern.surface_mangle_name());
            let payload_fields = self
                .lookup_variant(variant_name)
                .map(|(_, _, f)| f)
                .unwrap_or(&[]);
            for (slot, (param_name, _)) in params.iter().enumerate() {
                if param_name.as_str() == "_" {
                    continue;
                }
                let field_mlir_ty = payload_fields
                    .get(slot)
                    .map(|(_, ty)| self.ty_to_mlir(ty))
                    .unwrap_or_else(|| self.mlir.i64());
                let default_ty = Ty::Int {
                    width: 64,
                    signed: true,
                    value: None,
                    min: None,
                    max: None,
                };
                let field_ty = payload_fields
                    .get(slot)
                    .map(|(_, ty)| ty)
                    .unwrap_or(&default_ty);
                let extracted = block.append_op(self.mlir.llvm_extractvalue(
                    subject_val,
                    (slot + 1) as i64,
                    field_mlir_ty,
                    self.location(),
                ));
                symtab.insert(*param_name, extracted, field_ty.clone(), false);
            }
        }
    }
}
