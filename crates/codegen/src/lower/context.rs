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
use melior::ir::{BlockRef, Value, ValueLike, operation::OperationBuilder};
use typecheck::ResolvedProgram;
use typecheck::VariantLookupResult;
use typecheck::layout::{Layout, LayoutKind, TargetLayout};
use typecheck::representation::Repr;
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
/// Each unique string gets an internal global symbol. The registry tracks
/// allocation count and provides name-lookup for deduplication.
pub struct StringRegistry {
    literals: RefCell<Vec<String>>,
    symbols: RefCell<HashMap<String, String>>,
    counter: Cell<usize>,
    prefix: String,
}

impl StringRegistry {
    pub fn new() -> Self {
        Self {
            literals: RefCell::new(Vec::new()),
            symbols: RefCell::new(HashMap::new()),
            counter: Cell::new(0),
            prefix: String::new(),
        }
    }

    pub(crate) fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            ..Self::new()
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
        let name = format!("__gin-string-{}{counter}", self.prefix);
        self.counter.set(counter + 1);

        self.symbols
            .borrow_mut()
            .insert(s.to_string(), name.clone());
        self.literals.borrow_mut().push(s.to_string());

        name
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
    pub program: ResolvedProgram<'a>,
    pub target_layout: TargetLayout,
    pub strings: StringRegistry,
    pub symptoms: SymptomCollector,
    pub current_span: Cell<SpanId>,
    pub source_map: CodegenSourceMap<'a>,
}

#[derive(Clone)]
pub struct CodegenSourceMap<'a> {
    pub filename: String,
    pub span_table: &'a SpanTable,
    pub line_starts: Vec<usize>,
    pub source_len: usize,
}

impl<'a> CodegenSourceMap<'a> {
    pub fn new(filename: impl Into<String>, source: &str, span_table: &'a SpanTable) -> Self {
        Self {
            filename: filename.into(),
            span_table,
            line_starts: source.compute_line_starts(),
            source_len: source.len(),
        }
    }
}

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(crate) fn with_string_prefix(
        mlir: &'c Context,
        program: ResolvedProgram<'a>,
        target_layout: TargetLayout,
        source_map: CodegenSourceMap<'a>,
        string_prefix: &str,
    ) -> Self {
        Self {
            mlir,
            program,
            target_layout,
            strings: StringRegistry::with_prefix(string_prefix),
            symptoms: SymptomCollector::new(),
            current_span: Cell::new(SpanId::INVALID),
            source_map,
        }
    }

    pub fn lookup_variant(&self, name: Intern<String>) -> Option<VariantLookupResult<'_>> {
        self.program.lookup_variant(&name)
    }

    pub fn location(&self) -> Location<'c> {
        let id = self.current_span.get();
        if !id.is_valid() {
            return Location::new(self.mlir, &self.source_map.filename, 0, 0);
        }
        let span = self.source_map.span_table.get(id);
        let byte = (span.start()).min(self.source_map.source_len);
        let (line, col) = self.source_map.line_starts.byte_offset_to_line_col(byte);
        Location::new(self.mlir, &self.source_map.filename, line, col)
    }

    pub fn register_string(&self, s: &str) -> String {
        self.strings.register(s)
    }

    pub fn emit_symptom(&self, code: &str, message: String) {
        self.symptoms.emit(
            Diagnostic::new(code, message)
                .at_span_id(self.current_span.get(), self.source_map.span_table),
        );
    }

    pub fn emit_internal(&self, message: impl Into<String>) {
        self.symptoms
            .emit_internal(message, self.current_span.get(), self.source_map.span_table);
    }

    pub fn validate_pointee_stride(&self, pointee: &Ty) -> bool {
        if self
            .target_layout
            .pointee_stride(pointee, Some(self.program.type_registry))
            .is_err()
        {
            self.emit_symptom(
                "raw-pointer-offset-layout",
                "raw-pointer offset requires a sized pointee layout".to_string(),
            );
            return false;
        }
        true
    }

    pub fn is_empty_product(&self, ty: &Ty) -> bool {
        let Ok(repr) =
            typecheck::representation::Repr::derive(ty, Some(self.program.type_registry))
        else {
            return match ty {
                Ty::Named { instance, .. } => self
                    .program
                    .type_registry
                    .declaration_for_instance(instance)
                    .and_then(|declaration| declaration.fixed_array.as_ref())
                    .is_none(),
                Ty::Unit => true,
                Ty::Tuple(fields) => fields.is_empty(),
                Ty::Record { fields, .. } => fields.is_empty(),
                _ => false,
            };
        };
        matches!(
            self.target_layout.layout_repr(&repr),
            Ok(typecheck::layout::Layout {
                abi: typecheck::layout::AbiClass::Empty,
                ..
            })
        )
    }

    pub fn scale_pointer_index(
        &self,
        block: &BlockRef<'c, 'c>,
        index: Value<'c, 'c>,
        pointee: &Ty,
    ) -> Option<Value<'c, 'c>> {
        let stride = self
            .target_layout
            .pointee_stride(pointee, Some(self.program.type_registry))
            .ok()?;
        if stride == 1 {
            return Some(index);
        }
        let location = self.location();
        let scale = block.const_int(self.mlir, index.r#type(), i128::from(stride), location);
        let operation = OperationBuilder::new("arith.muli", location)
            .add_operands(&[index, scale])
            .add_results(&[index.r#type()])
            .build()
            .ok()?;
        Some(block.append_op(operation))
    }

    pub fn sum_layout(&self, ty: &Ty) -> Option<(Repr, Layout)> {
        let repr = match Repr::derive(ty, Some(self.program.type_registry)) {
            Ok(repr) => repr,
            Err(error) => {
                self.emit_symptom(error.code(), error.to_string());
                return None;
            }
        };
        if !matches!(repr, Repr::Sum { .. }) {
            self.emit_symptom(
                "sum-representation-missing",
                "expected a tagged-sum representation".to_string(),
            );
            return None;
        }
        let layout = match self.target_layout.layout_repr(&repr) {
            Ok(layout) => layout,
            Err(error) => {
                self.emit_symptom(error.code(), error.to_string());
                return None;
            }
        };
        Some((repr, layout))
    }

    pub fn sum_discriminant_i64(
        &self,
        block: &BlockRef<'c, 'c>,
        subject: Value<'c, 'c>,
        subject_ty: &Ty,
    ) -> Option<Value<'c, 'c>> {
        let (_, layout) = self.sum_layout(subject_ty)?;
        let LayoutKind::Sum {
            discriminant,
            payload_size,
            ..
        } = &layout.kind
        else {
            return None;
        };
        let discriminant_ty = self.repr_to_mlir(discriminant);
        let value = if *payload_size == 0 {
            subject
        } else {
            block.append_op(self.mlir.llvm_extractvalue(
                subject,
                0,
                discriminant_ty,
                self.location(),
            ))
        };
        let Repr::Bits { width } = discriminant else {
            self.emit_symptom(
                "sum-representation-missing",
                "tagged-sum discriminant is not an integer representation".to_string(),
            );
            return None;
        };
        if *width == 64 {
            return Some(value);
        }
        let operation = OperationBuilder::new("arith.extui", self.location())
            .add_operands(&[value])
            .add_results(&[self.mlir.i64()])
            .build()
            .ok()?;
        Some(block.append_op(operation))
    }

    pub fn lower_sum_constructor(
        &self,
        block: &BlockRef<'c, 'c>,
        sum_ty: &Ty,
        discriminant: usize,
        args: &[Value<'c, 'c>],
    ) -> Option<Value<'c, 'c>> {
        let (repr, layout) = self.sum_layout(sum_ty)?;
        let LayoutKind::Sum {
            discriminant: discriminant_repr,
            payload_offset,
            payload_size,
            variant_layouts,
            ..
        } = &layout.kind
        else {
            return None;
        };
        let Repr::Sum { variants } = &repr else {
            return None;
        };
        let Some(variant_repr) = variants.get(discriminant) else {
            self.emit_symptom(
                "invalid-variant-discriminant",
                "tagged-sum constructor discriminant is out of range".to_string(),
            );
            return None;
        };
        let Repr::Product {
            fields: field_reprs,
        } = variant_repr
        else {
            return None;
        };
        if field_reprs.len() != args.len() {
            self.emit_symptom(
                "sum-payload-arity-mismatch",
                "tagged-sum constructor payload arity does not match its variant".to_string(),
            );
            return None;
        }
        let discriminant_ty = self.repr_to_mlir(discriminant_repr);
        let discriminant_value = block.const_int(
            self.mlir,
            discriminant_ty,
            discriminant as i128,
            self.location(),
        );
        if *payload_size == 0 {
            return Some(discriminant_value);
        }
        let sum_mlir_ty = self.repr_to_mlir(&repr);
        let storage = block.alloca_typed(self.mlir, sum_mlir_ty, self.location());
        block.store_typed(self, storage, discriminant_value, self.location())?;
        let variant_layout = variant_layouts.get(discriminant)?;
        let LayoutKind::Product { field_offsets } = &variant_layout.kind else {
            return None;
        };
        for ((arg, _field_repr), field_offset) in args.iter().zip(field_reprs).zip(field_offsets) {
            let offset = block.const_i64(
                self.mlir,
                (*payload_offset + *field_offset) as i64,
                self.location(),
            );
            let field_ptr = block.gep_i8(self, storage, offset, self.location())?;
            block.store_typed(self, field_ptr, *arg, self.location())?;
        }
        block.load_typed(self, storage, sum_mlir_ty, self.location())
    }

    pub fn drain_symptoms(&self) -> Vec<Diagnostic> {
        self.symptoms.drain()
    }

    /// Extend a discriminant to i64 for comparison.
    pub fn emit_discriminant_extend(
        &self,
        block: &BlockRef<'c, 'c>,
        subject: Value<'c, 'c>,
        variant_count: usize,
    ) -> Value<'c, 'c> {
        let loc = self.location();
        if variant_count <= 256 {
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
        subject_ty: &Ty,
        symtab: &mut ScopedSymbolTable<'c>,
    ) {
        match pattern {
            Pattern::Nominal(name, _)
                if name.as_str() != "_"
                    && name
                        .as_str()
                        .chars()
                        .next()
                        .is_some_and(|character| character.is_ascii_lowercase()) =>
            {
                symtab.insert(*name, subject_val, subject_ty.clone(), false);
            }
            Pattern::Generic { params, .. } => {
                let variant_name = Intern::<String>::from_ref(pattern.surface_mangle_name());
                let Some((_union, variant_index, _lookup_fields)) =
                    self.lookup_variant(variant_name)
                else {
                    self.emit_symptom(
                        "invalid-field-projection",
                        "cannot resolve tagged-sum payload fields".to_string(),
                    );
                    return;
                };
                let Some((repr, layout)) = self.sum_layout(subject_ty) else {
                    return;
                };
                let LayoutKind::Sum {
                    payload_offset,
                    variant_layouts,
                    ..
                } = &layout.kind
                else {
                    return;
                };
                let Some(variant_layout) = variant_layouts.get(variant_index) else {
                    return;
                };
                let LayoutKind::Product { field_offsets } = &variant_layout.kind else {
                    return;
                };
                let Repr::Sum { variants } = &repr else {
                    return;
                };
                let Some(Repr::Product {
                    fields: field_reprs,
                }) = variants.get(variant_index)
                else {
                    return;
                };
                let resolved_subject = self
                    .program
                    .type_registry
                    .resolved_definition_for_type(subject_ty);
                let Some(payload_fields) = (match &resolved_subject {
                    Ty::Union { variants, .. } => variants
                        .get(variant_index)
                        .map(|variant| variant.fields.as_slice()),
                    _ => None,
                }) else {
                    self.emit_symptom(
                        "invalid-field-projection",
                        "tagged-sum payload fields are unavailable for this application"
                            .to_string(),
                    );
                    return;
                };
                let storage = block.alloca_typed(self.mlir, subject_val.r#type(), self.location());
                if block
                    .store_typed(self, storage, subject_val, self.location())
                    .is_none()
                {
                    return;
                }
                for (slot, (param_name, _)) in params.iter().enumerate() {
                    if param_name.as_str() == "_" {
                        continue;
                    }
                    let Some((_, field_ty)) = payload_fields.get(slot) else {
                        self.emit_symptom(
                            "invalid-field-projection",
                            "tagged-sum payload field is missing".to_string(),
                        );
                        continue;
                    };
                    let Some(field_offset) = field_offsets.get(slot) else {
                        continue;
                    };
                    let Some(field_repr) = field_reprs.get(slot) else {
                        continue;
                    };
                    let _ = field_repr;
                    let offset = block.const_i64(
                        self.mlir,
                        (*payload_offset + *field_offset) as i64,
                        self.location(),
                    );
                    let Some(field_ptr) = block.gep_i8(self, storage, offset, self.location())
                    else {
                        continue;
                    };
                    let Some(extracted) = block.load_typed(
                        self,
                        field_ptr,
                        self.ty_to_mlir(field_ty),
                        self.location(),
                    ) else {
                        continue;
                    };
                    symtab.insert(*param_name, extracted, field_ty.as_ref().clone(), false);
                }
            }
            _ => {}
        }
    }
}
