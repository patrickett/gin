//! Codegen context carrying MLIR state, type info, and symptom collection.
//! This module provides:
//!
//! - [`CodegenContext`] — the central context object shared across all lowering
//! - Helper functions for computing line/column positions in source

use super::frame::FunctionStorage;
use super::return_::ReturnStorage;
use diagnostic::code::{CodegenCode, LayoutCode, RepresentationCode};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    fmt,
};

use crate::mlir_ext::{BlockExt, OperationBuilderExt};
use ::span::{SpanId, SpanTable};
use ast::source::{LineStartsExt, SourceExt};
use diagnostic::{Diagnostic, DiagnosticCode};
use internment::Intern;
use melior::Context;
use melior::ir::Location;
use melior::ir::{BlockRef, Value, ValueLike};
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
            // codegen-internal
            Diagnostic::new(CodegenCode::Internal, message.into())
                .with_help("an internal compiler error occurred")
                .at_span(span_table.get(span)),
        );
    }

    pub(super) fn count(&self) -> usize {
        self.symptoms.borrow().len()
    }

    pub fn drain(&self) -> Vec<Diagnostic> {
        self.symptoms.borrow_mut().drain(..).collect()
    }
}

impl fmt::Debug for SymptomCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SymptomCollector")
            .field("count", &self.symptoms.borrow().len())
            .finish()
    }
}

/// Interns module-level constants during lowering: string literals and promoted
/// arrays. Symbols are assigned deterministically per kind in first-use order.
pub(crate) struct GlobalConstants {
    prefix: String,
    strings: RefCell<HashMap<String, usize>>,
    arrays: RefCell<Vec<(Ty, ast::ConstValue)>>,
}

impl GlobalConstants {
    pub(crate) fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            strings: RefCell::new(HashMap::new()),
            arrays: RefCell::new(Vec::new()),
        }
    }

    pub(crate) fn intern_string(&self, value: &str) -> String {
        let mut symbols = self.strings.borrow_mut();
        let next = symbols.len();
        let index = match symbols.get(value) {
            Some(&index) => index,
            None => {
                symbols.insert(value.to_owned(), next);
                next
            }
        };
        format!("__gin-string-{}{index}", self.prefix)
    }

    pub(crate) fn intern_array(&self, ty: &Ty, value: &ast::ConstValue) -> String {
        let mut arrays = self.arrays.borrow_mut();
        let index = match arrays
            .iter()
            .position(|(existing_ty, existing)| existing_ty == ty && existing == value)
        {
            Some(index) => index,
            None => {
                arrays.push((ty.clone(), value.clone()));
                arrays.len() - 1
            }
        };
        format!("__gin-array-{}{index}", self.prefix)
    }

    /// `(symbol, value)` pairs in first-use order, for emission and tests.
    pub(crate) fn string_symbols(&self) -> Vec<(String, String)> {
        let symbols = self.strings.borrow();
        let mut entries: Vec<_> = symbols.iter().collect();
        entries.sort_unstable_by_key(|(_, index)| **index);
        entries
            .into_iter()
            .map(|(value, &index)| {
                (
                    format!("__gin-string-{}{index}", self.prefix),
                    value.clone(),
                )
            })
            .collect()
    }

    /// `(symbol, ty, value)` entries in first-use order.
    pub(crate) fn for_each_array(&self, mut visit: impl FnMut(String, &Ty, &ast::ConstValue)) {
        for (index, (ty, value)) in self.arrays.borrow().iter().enumerate() {
            visit(format!("__gin-array-{}{index}", self.prefix), ty, value);
        }
    }
}

pub struct CodegenContext<'a, 'c> {
    pub mlir: &'c Context,
    pub program: ResolvedProgram<'a>,
    pub target_layout: TargetLayout,
    pub(super) empty_product_cache: RefCell<HashMap<Ty, bool>>,
    pub(super) type_cache: RefCell<HashMap<Ty, melior::ir::Type<'c>>>,
    pub(super) addressed_binders: HashSet<ast::BinderId>,
    pub(super) emission_construction: HashSet<typecheck::ExprId>,
    pub(crate) globals: GlobalConstants,
    pub symptoms: SymptomCollector,
    pub current_span: Cell<SpanId>,
    pub source_map: CodegenSourceMap<'a>,
    pub(super) when_subjects: RefCell<HashMap<typecheck::ExprId, Value<'c, 'c>>>,
    pub(super) operand_values: RefCell<HashMap<typecheck::ExprId, crate::LoweredValue<'c>>>,
    pub(super) active_operands: RefCell<HashSet<typecheck::ExprId>>,
    pub(super) return_storage: RefCell<Option<ReturnStorage<'c>>>,
    pub(super) function_storage: RefCell<Option<FunctionStorage<'c>>>,
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
        let symptoms = SymptomCollector::new();
        let addressed_binders = program
            .exprs
            .kind
            .iter()
            .enumerate()
            .filter_map(|(index, kind)| {
                let (typecheck::TypedExprKind::Ref(inner)
                | typecheck::TypedExprKind::TakePtr(inner)) = kind
                else {
                    return None;
                };
                let mut place = program.exprs.place_of(*inner).flatten()?;
                let invalid = |message: &str| {
                    let span = program
                        .exprs
                        .span
                        .get(index)
                        .copied()
                        .unwrap_or(SpanId::INVALID);
                    symptoms.emit(
                        // codegen-addressed-place-invalid
                        Diagnostic::new(CodegenCode::AddressedPlaceInvalid, message)
                            .at_span(source_map.span_table.get(span)),
                    );
                    None
                };
                loop {
                    let Some(info) = program.places.get(place.0 as usize) else {
                        return invalid("addressed expression refers to a missing place");
                    };
                    match info.parent {
                        Some(parent) if parent.0 < place.0 => place = parent,
                        Some(_) => return invalid("projected place parent must precede its child"),
                        None => return Some(info.binder),
                    }
                }
            })
            .collect();
        Self {
            mlir,
            program,
            target_layout,
            empty_product_cache: RefCell::new(HashMap::new()),
            type_cache: RefCell::new(HashMap::new()),
            addressed_binders,
            emission_construction: program
                .exprs
                .kind
                .iter()
                .filter_map(|kind| match kind {
                    typecheck::TypedExprKind::Emit {
                        state: typecheck::assembly::AssemblyState::Checked(assembly),
                        ..
                    } => Some(assembly.construction.iter().copied()),
                    _ => None,
                })
                .flatten()
                .collect(),
            globals: GlobalConstants::with_prefix(string_prefix),
            symptoms,
            current_span: Cell::new(SpanId::INVALID),
            source_map,
            when_subjects: RefCell::new(HashMap::new()),
            operand_values: RefCell::new(HashMap::new()),
            active_operands: RefCell::new(HashSet::new()),
            return_storage: RefCell::new(None),
            function_storage: RefCell::new(None),
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

    pub fn emit_symptom(&self, code: DiagnosticCode, message: String) {
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
            // raw-pointer-offset-layout
            self.emit_symptom(
                DiagnosticCode::Layout(LayoutCode::RawPointerOffset),
                "raw-pointer offset requires a sized pointee layout".to_string(),
            );
            return false;
        }
        true
    }

    pub fn is_empty_product(&self, ty: &Ty) -> bool {
        if let Some(empty) = self.empty_product_cache.borrow().get(ty).copied() {
            return empty;
        }
        let empty = self.uncached_is_empty_product(ty);
        self.empty_product_cache
            .borrow_mut()
            .insert(ty.clone(), empty);
        empty
    }

    fn uncached_is_empty_product(&self, ty: &Ty) -> bool {
        let Ok(repr) =
            typecheck::representation::Repr::derive(ty, Some(self.program.type_registry))
        else {
            return false;
        };
        matches!(
            self.target_layout.layout_repr(&repr),
            Ok(typecheck::layout::Layout {
                abi: typecheck::layout::AbiClass::Empty,
                ..
            })
        )
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
            // sum-representation-missing
            self.emit_symptom(
                DiagnosticCode::Representation(RepresentationCode::SumMissing),
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

    pub fn sum_discriminant(
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
        let Repr::Bits { .. } = discriminant else {
            // sum-representation-missing
            self.emit_symptom(
                DiagnosticCode::Representation(RepresentationCode::SumMissing),
                "tagged-sum discriminant is not an integer representation".to_string(),
            );
            return None;
        };
        Some(value)
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
        let Repr::Sum { variants, .. } = &repr else {
            return None;
        };
        let Some(variant_repr) = variants.get(discriminant) else {
            // invalid-variant-discriminant
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::VariantDiscriminantInvalid),
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
            // sum-payload-arity-mismatch
            self.emit_symptom(
                DiagnosticCode::Codegen(CodegenCode::SumPayloadArityMismatch),
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
        let storage = self.allocate_stack_slot(sum_mlir_ty)?;
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

    /// Shared between `if_` pattern conditions and `when` pattern arms.
    /// Bind pattern payload fields from a subject value into the symbol table.
    pub(crate) fn sum_payload_offset(&self, ty: &Ty, variant: usize, field: usize) -> Option<u64> {
        let (_, layout) = self.sum_layout(ty)?;
        let LayoutKind::Sum {
            payload_offset,
            variant_layouts,
            ..
        } = &layout.kind
        else {
            return None;
        };
        let LayoutKind::Product { field_offsets } = &variant_layouts.get(variant)?.kind else {
            return None;
        };
        Some(*payload_offset + *field_offsets.get(field)?)
    }

    pub(crate) fn lower_sum_payload(
        &self,
        block: &BlockRef<'c, 'c>,
        value: Value<'c, 'c>,
        sum_ty: &Ty,
        variant: usize,
        field: usize,
        field_ty: &Ty,
    ) -> Option<Value<'c, 'c>> {
        let offset = self.sum_payload_offset(sum_ty, variant, field)?;
        let storage = self.allocate_stack_slot(value.r#type())?;
        block.store_typed(self, storage, value, self.location())?;
        let offset = block.const_i64(self.mlir, offset as i64, self.location());
        let ptr = block.gep_i8(self, storage, offset, self.location())?;
        block.load_typed(self, ptr, self.ty_to_mlir(field_ty), self.location())
    }
}

#[cfg(test)]
mod global_constants_tests {
    use super::GlobalConstants;
    use ast::Ty;

    #[test]
    fn globals_follow_registration_order_beyond_single_digit_indices() {
        let registry = GlobalConstants::with_prefix("2_");
        let expected: Vec<_> = (0..12)
            .map(|index| {
                let value = format!("literal-{index}");
                let symbol = format!("__gin-string-2_{index}");
                assert_eq!(registry.intern_string(&value), symbol);
                (symbol, value)
            })
            .collect();
        assert_eq!(registry.string_symbols(), expected);
    }

    #[test]
    fn duplicate_literals_do_not_allocate_symbols() {
        let registry = GlobalConstants::with_prefix("");
        assert!(registry.string_symbols().is_empty());
        assert_eq!(registry.intern_string(""), "__gin-string-0");
        assert_eq!(registry.intern_string("é"), "__gin-string-1");
        assert_eq!(registry.intern_string(""), "__gin-string-0");
        assert_eq!(registry.intern_string("é"), "__gin-string-1");
        assert_eq!(registry.intern_string("next"), "__gin-string-2");
        assert_eq!(registry.string_symbols().len(), 3);
    }

    #[test]
    fn duplicate_arrays_reuse_symbols_and_keep_first_use_order() {
        let registry = GlobalConstants::with_prefix("");
        let value = ast::ConstValue::Int(7.into());
        assert_eq!(registry.intern_array(&Ty::Unit, &value), "__gin-array-0");
        assert_eq!(registry.intern_array(&Ty::Unit, &value), "__gin-array-0");
        assert_eq!(registry.intern_array(&Ty::i64(), &value), "__gin-array-1");
        let mut names = Vec::new();
        registry.for_each_array(|name, _, _| names.push(name));
        assert_eq!(names, ["__gin-array-0", "__gin-array-1"]);
    }
}
