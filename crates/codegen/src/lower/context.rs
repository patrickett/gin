//! Codegen context carrying MLIR state, type info, and symptom collection.
//! This module provides:
//!
//! - [`TypeInfo`] — information about a type's min/max range and bit width
//! - [`CodegenContext`] — the central context object shared across all lowering
//! - Helper functions for computing line/column positions in source

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    fmt,
};

use crate::ScopedSymbolTable;
use crate::mlir_ext::{BlockExt, ContextExt, OperationBuilderExt};
use ::span::{SpanId, SpanTable};
use ast::source::{LineStartsExt, SourceExt};
use ast::{Bind, SymbolTable as CompileTimeSymbolTable, TypeExpr};
use diagnostic::Diagnostic;
use i256::I256;
use internment::Intern;
use melior::Context;
use melior::ir::Location;
use melior::ir::{BlockRef, Value, operation::OperationBuilder};
use typecheck::VariantLookupResult;
use typecheck::analysis::{LocalTypes, TyInferEnv};
use typecheck::compile_time_trait::CompileTimeTraitRegistry;
use typecheck::ty::Ty;
use typecheck::{DefId, TagId, TypedFileAst};

#[derive(Debug, Clone)]
pub struct TypeInfo {
    pub min: I256,
    pub max: I256,
}

impl TypeInfo {
    pub fn bit_width(&self) -> u32 {
        let range = self.max - self.min;
        if range <= I256::from_i128(u8::MAX as i128 + 1) {
            8
        } else if range <= I256::from_i128(u16::MAX as i128 + 1) {
            16
        } else if range <= I256::from_i128(u32::MAX as i128 + 1) {
            32
        } else if range <= I256::from_i128(u64::MAX as i128 + 1) {
            64
        } else {
            128
        }
    }
}

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

    /// All registered string literals for emitting as globals.
    pub fn literals(&self) -> Vec<String> {
        self.literals.borrow().clone()
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
    pub type_info: &'a HashMap<Intern<String>, TypeInfo>,
    pub symbol_table: &'a CompileTimeSymbolTable,
    /// Intern<String>-keyed tag types (converted from TagId-keyed TypedFileAst on construction).
    pub tag_types: HashMap<Intern<String>, Ty>,
    /// Intern<String>-keyed fn return types (converted from DefId-keyed TypedFileAst on construction).
    pub fn_return_types: HashMap<Intern<String>, Ty>,
    pub strings: StringRegistry,
    /// Maps variable name → its resolved Ty, used for field-access lowering.
    pub var_types: RefCell<HashMap<Intern<String>, Ty>>,
    /// Names of mutable (`:`) local variables — their symtab value is an alloca ptr.
    /// Cleared at the start of each top-level function lower.
    pub mutable_slots: RefCell<HashSet<String>>,
    /// Element type of global constant arrays (top-level `:=` TupleLit binds), keyed by name.
    pub global_const_elems: RefCell<HashMap<String, Ty>>,
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
        type_info: &'a HashMap<Intern<String>, TypeInfo>,
        symbol_table: &'a CompileTimeSymbolTable,
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
            type_info,
            symbol_table,
            strings: StringRegistry::new(),
            var_types: RefCell::new(HashMap::new()),
            mutable_slots: RefCell::new(HashSet::new()),
            global_const_elems: RefCell::new(HashMap::new()),
            symptoms: SymptomCollector::new(),
            current_span: Cell::new(SpanId::INVALID),
            source_filename: filename.to_string(),
            source,
            span_table,
            line_starts: source.compute_line_starts(),
        }
    }

    pub fn lookup_tag(&self, name: Intern<String>) -> Option<&Ty> {
        self.typed_ast
            .and_then(|typed| typed.tag_types.get(&TagId(name)))
    }

    pub fn lookup_variant(&self, name: Intern<String>) -> Option<VariantLookupResult<'_>> {
        self.typed_ast
            .and_then(|typed| typed.variant_map.get(&name))
            .and_then(|candidates| candidates.first())
            .map(|(union, idx, fields)| (*union, *idx, fields.as_slice()))
    }

    pub fn fn_return_ty(&self, name: &Intern<String>) -> Option<&Ty> {
        self.typed_ast
            .and_then(|typed| typed.fn_return_types.get(&DefId(*name)))
    }

    pub fn infer_env(&'a self, locals: &'a dyn LocalTypes) -> TyInferEnv<'a> {
        TyInferEnv {
            tag_types: &self.tag_types,
            fn_return_types: &self.fn_return_types,
            locals,
            tag_params: None,
        }
    }

    pub fn resolve_type_surface(&self, e: &TypeExpr) -> Option<Ty> {
        e.is_type_surface()
            .then(|| typecheck::analysis::resolve_type_expr_from_map(e, &self.tag_types))
    }

    pub fn param_types<'b>(&self, bind: &'b Bind) -> Vec<(&'b Intern<String>, Ty)> {
        match bind.params.as_ref() {
            None => vec![],
            Some(params) => {
                let subst = bind
                    .receiver_type_surface()
                    .map(|sp| sp.value.typevars_from_receiver())
                    .unwrap_or_default();
                params
                    .iter()
                    .map(|(name, kind)| {
                        let ty = typecheck::analysis::resolve_parameter_kind_with_subst(
                            *name,
                            kind,
                            &self.tag_types,
                            &self.fn_return_types,
                            &subst,
                            None,
                        );
                        (name, ty)
                    })
                    .collect()
            }
        }
    }

    pub fn location(&self) -> Location<'c> {
        let id = self.current_span.get();
        if !id.is_valid() {
            return self.mlir.unknown_loc();
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
        pattern: &TypeExpr,
        subject_val: Value<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) {
        if let TypeExpr::Generic { params, .. } = pattern {
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
                let extracted = block.append_op(self.mlir.llvm_extractvalue(
                    subject_val,
                    (slot + 1) as i64,
                    field_mlir_ty,
                ));
                symtab.insert(param_name.as_str().to_string(), extracted);
            }
        }
    }
}

impl LocalTypes for CodegenContext<'_, '_> {
    fn get_type(&self, name: &Intern<String>) -> Option<Ty> {
        self.var_types.borrow().get(name).cloned()
    }
}
