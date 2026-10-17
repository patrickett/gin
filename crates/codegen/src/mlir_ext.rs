//! Extension traits for ergonomic Melior operations.
//!
//! This module provides extension traits that simplify common Melior patterns

use crate::prelude::*;
use melior::dialect::arith::CmpiPredicate;
use melior::ir::OperationRef;

/// Extension trait for [`melior::Context`] to simplify common operations.
pub trait ContextExt {
    fn i64(&self) -> Type<'_>;
    fn i1(&self) -> Type<'_>;
    fn f64(&self) -> Type<'_>;
    /// LLVM opaque pointer type
    fn llvm_ptr(&self) -> Type<'_>;
}

impl ContextExt for Context {
    fn i64(&self) -> Type<'_> {
        Type::from(IntegerType::new(self, 64))
    }

    fn i1(&self) -> Type<'_> {
        Type::from(IntegerType::new(self, 1))
    }

    fn f64(&self) -> Type<'_> {
        Type::float64(self)
    }

    fn llvm_ptr(&self) -> Type<'_> {
        r#type::pointer(self, 0)
    }
}

/// Extension trait for creating MLIR attributes.
pub trait AttributeExt<'c> {
    fn i64_attr(&self, value: i64) -> Attribute<'c>;
    fn f64_attr(&self, value: f64) -> Attribute<'c>;
    fn str_attr(&self, value: &str) -> Attribute<'c>;
    fn symbol_ref_attr(&self, name: &str) -> Attribute<'c>;
}

impl<'c> AttributeExt<'c> for &'c Context {
    fn i64_attr(&self, value: i64) -> Attribute<'c> {
        IntegerAttribute::new(self.i64(), value).into()
    }

    fn f64_attr(&self, value: f64) -> Attribute<'c> {
        FloatAttribute::new(self, self.f64(), value).into()
    }

    fn str_attr(&self, value: &str) -> Attribute<'c> {
        StringAttribute::new(self, value).into()
    }

    fn symbol_ref_attr(&self, name: &str) -> Attribute<'c> {
        FlatSymbolRefAttribute::new(self, name).into()
    }
}

/// Extension trait for building MLIR operations.
pub trait OperationBuilderExt<'c> {
    fn i64_const(&self, value: i64, loc: Location<'c>) -> Operation<'c>;
    /// Create an integer constant of arbitrary bit-width.
    fn int_const(&self, ty: Type<'c>, value: i128, loc: Location<'c>) -> Operation<'c>;
    fn const_op(&self, attr: Attribute<'c>, ty: Type<'c>, loc: Location<'c>) -> Operation<'c>;
    fn build_binop(
        &self,
        name: &'static str,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        result_ty: Type<'c>,
        loc: Location<'c>,
    ) -> Operation<'c>;
    fn build_cmpi(
        &self,
        predicate: u64,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        loc: Location<'c>,
    ) -> Operation<'c>;
    /// `llvm.mlir.undef` — produce an undefined value of the given type
    fn llvm_undef(&self, ty: Type<'c>, loc: Location<'c>) -> Operation<'c>;
    /// `llvm.insertvalue` — insert a value into an aggregate at `position`
    fn llvm_insertvalue(
        &self,
        container: Value<'c, 'c>,
        value: Value<'c, 'c>,
        position: i64,
        loc: Location<'c>,
    ) -> Operation<'c>;
    /// `llvm.extractvalue` — extract a value from an aggregate at `position`
    fn llvm_extractvalue(
        &self,
        container: Value<'c, 'c>,
        position: i64,
        result_type: Type<'c>,
        loc: Location<'c>,
    ) -> Operation<'c>;
}

impl<'c> OperationBuilderExt<'c> for &'c Context {
    fn i64_const(&self, value: i64, loc: Location<'c>) -> Operation<'c> {
        let ty = self.i64();
        self.const_op(IntegerAttribute::new(ty, value).into(), ty, loc)
    }

    fn int_const(&self, ty: Type<'c>, value: i128, loc: Location<'c>) -> Operation<'c> {
        let width = IntegerType::try_from(ty)
            .map(|integer| integer.width())
            .unwrap_or(128);
        // MLIR's i64 constructor zero-extends into wider signless integers.
        let native_attribute_preserves_bits = width <= 64 || value >= 0;
        let attr: Attribute<'c> = if native_attribute_preserves_bits
            && value >= i64::MIN as i128
            && value <= i64::MAX as i128
        {
            IntegerAttribute::new(ty, value as i64).into()
        } else {
            let source = format!("{value} : {ty}");
            Attribute::parse(self, &source).expect("integer constant must fit its representation")
        };
        self.const_op(attr, ty, loc)
    }

    fn const_op(&self, attr: Attribute<'c>, ty: Type<'c>, loc: Location<'c>) -> Operation<'c> {
        let value_id = Identifier::new(self, "value");
        OperationBuilder::new("arith.constant", loc)
            .add_attributes(&[(value_id, attr)])
            .add_results(&[ty])
            .build()
            .expect("arith.constant build should succeed")
    }

    fn build_binop(
        &self,
        name: &'static str,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        result_ty: Type<'c>,
        loc: Location<'c>,
    ) -> Operation<'c> {
        OperationBuilder::new(name, loc)
            .add_operands(&[lhs, rhs])
            .add_results(&[result_ty])
            .build()
            .expect("arith.constant build should succeed")
    }

    fn build_cmpi(
        &self,
        predicate: u64,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        loc: Location<'c>,
    ) -> Operation<'c> {
        let pred_attr = self.i64_attr(predicate as i64);
        let pred_id = Identifier::new(self, "predicate");
        OperationBuilder::new("arith.cmpi", loc)
            .add_attributes(&[(pred_id, pred_attr)])
            .add_operands(&[lhs, rhs])
            .add_results(&[self.i1()])
            .build()
            .expect("arith.cmpi build should succeed")
    }

    fn llvm_undef(&self, ty: Type<'c>, loc: Location<'c>) -> Operation<'c> {
        melior::dialect::llvm::undef(ty, loc)
    }

    fn llvm_insertvalue(
        &self,
        container: Value<'c, 'c>,
        value: Value<'c, 'c>,
        position: i64,
        loc: Location<'c>,
    ) -> Operation<'c> {
        melior::dialect::llvm::insert_value(
            self,
            container,
            DenseI64ArrayAttribute::new(self, &[position]),
            value,
            loc,
        )
    }

    fn llvm_extractvalue(
        &self,
        container: Value<'c, 'c>,
        position: i64,
        result_type: Type<'c>,
        loc: Location<'c>,
    ) -> Operation<'c> {
        melior::dialect::llvm::extract_value(
            self,
            container,
            DenseI64ArrayAttribute::new(self, &[position]),
            result_type,
            loc,
        )
    }
}

/// Extension trait for [`melior::ir::Block`] to simplify appending operations.
pub trait BlockExt<'c> {
    fn append_op(&self, op: Operation<'c>) -> Value<'c, 'c>;
    fn append_scf_if(
        &self,
        condition: Value<'c, 'c>,
        result_types: &[Type<'c>],
        then_region: Region<'c>,
        else_region: Region<'c>,
        loc: Location<'c>,
    ) -> OperationRef<'c, 'c>;
    fn const_i64(&self, ctx: &'c Context, value: i64, loc: Location<'c>) -> Value<'c, 'c>;
    /// Create an integer constant of the given MLIR type.
    fn const_int(
        &self,
        ctx: &'c Context,
        ty: Type<'c>,
        value: i128,
        loc: Location<'c>,
    ) -> Value<'c, 'c>;
    /// Create a string constant - returns a fat pointer (ptr, length)
    /// This requires access to CodegenContext to register the string globally
    fn unit_value(&self, ctx: &CodegenContext<'_, 'c>, loc: Location<'c>) -> Value<'c, 'c>;
    fn call(
        &self,
        ctx: &'c Context,
        func_name: &str,
        args: &[Value<'c, 'c>],
        return_type: Type<'c>,
        loc: Location<'c>,
    ) -> Value<'c, 'c>;
    fn call_unit(
        &self,
        ctx: &'c Context,
        func_name: &str,
        args: &[Value<'c, 'c>],
        loc: Location<'c>,
    );
    fn gep_typed(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        base: Value<'c, 'c>,
        elem_ty: Type<'c>,
        indices: &[Value<'c, 'c>],
        raw_indices: &[i32],
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>>;
    #[allow(clippy::too_many_arguments)]
    fn gep_typed_result(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        base: Value<'c, 'c>,
        elem_ty: Type<'c>,
        indices: &[Value<'c, 'c>],
        raw_indices: &[i32],
        result_ty: Type<'c>,
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>>;
    /// `llvm.getelementptr` with a dynamic byte offset into a `!llvm.ptr`.
    fn gep_i8(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        base: Value<'c, 'c>,
        idx: Value<'c, 'c>,
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>>;
    /// `llvm.load elem_ty, ptr` — load a typed value from a pointer.
    fn load_typed(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        ptr: Value<'c, 'c>,
        elem_ty: Type<'c>,
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>>;
    /// `llvm.store val, ptr` — store a value through a pointer.
    fn store_typed(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        ptr: Value<'c, 'c>,
        val: Value<'c, 'c>,
        loc: Location<'c>,
    ) -> Option<()>;
}

impl<'c> BlockExt<'c> for BlockRef<'c, 'c> {
    fn append_op(&self, op: Operation<'c>) -> Value<'c, 'c> {
        let op_ref = self.append_operation(op);
        op_ref
            .result(0)
            .expect("append_operation result 0 should exist")
            .into()
    }

    fn append_scf_if(
        &self,
        condition: Value<'c, 'c>,
        result_types: &[Type<'c>],
        then_region: Region<'c>,
        else_region: Region<'c>,
        loc: Location<'c>,
    ) -> OperationRef<'c, 'c> {
        self.append_operation(scf_dialect::r#if(
            condition,
            result_types,
            then_region,
            else_region,
            loc,
        ))
    }

    fn const_i64(&self, ctx: &'c Context, value: i64, loc: Location<'c>) -> Value<'c, 'c> {
        self.append_op(ctx.i64_const(value, loc))
    }

    fn const_int(
        &self,
        ctx: &'c Context,
        ty: Type<'c>,
        value: i128,
        loc: Location<'c>,
    ) -> Value<'c, 'c> {
        self.append_op(ctx.int_const(ty, value, loc))
    }

    fn unit_value(&self, ctx: &CodegenContext<'_, 'c>, loc: Location<'c>) -> Value<'c, 'c> {
        let unit_ty = r#type::r#struct(ctx.mlir, &[], false);
        self.append_op(ctx.mlir.llvm_undef(unit_ty, loc))
    }

    fn call(
        &self,
        ctx: &'c Context,
        func_name: &str,
        args: &[Value<'c, 'c>],
        return_type: Type<'c>,
        loc: Location<'c>,
    ) -> Value<'c, 'c> {
        let callee_id = Identifier::new(ctx, "callee");
        let symbol_ref = ctx.symbol_ref_attr(func_name);
        self.append_op(
            OperationBuilder::new("func.call", loc)
                .add_attributes(&[(callee_id, symbol_ref)])
                .add_operands(args)
                .add_results(&[return_type])
                .build()
                .expect("func.call build should succeed"),
        )
    }

    fn call_unit(
        &self,
        ctx: &'c Context,
        func_name: &str,
        args: &[Value<'c, 'c>],
        loc: Location<'c>,
    ) {
        let callee_id = Identifier::new(ctx, "callee");
        let symbol_ref = ctx.symbol_ref_attr(func_name);
        let _ = self.append_operation(
            OperationBuilder::new("func.call", loc)
                .add_attributes(&[(callee_id, symbol_ref)])
                .add_operands(args)
                .build()
                .expect("func.call build should succeed"),
        );
    }

    fn gep_typed(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        base: Value<'c, 'c>,
        elem_ty: Type<'c>,
        indices: &[Value<'c, 'c>],
        raw_indices: &[i32],
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>> {
        self.gep_typed_result(
            cctx,
            base,
            elem_ty,
            indices,
            raw_indices,
            cctx.mlir.llvm_ptr(),
            loc,
        )
    }

    fn gep_typed_result(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        base: Value<'c, 'c>,
        elem_ty: Type<'c>,
        indices: &[Value<'c, 'c>],
        raw_indices: &[i32],
        result_ty: Type<'c>,
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>> {
        let ctx = cctx.mlir;
        let mut operands = Vec::with_capacity(indices.len() + 1);
        operands.push(base);
        operands.extend_from_slice(indices);
        let op = OperationBuilder::new("llvm.getelementptr", loc)
            .add_attributes(&[
                (
                    Identifier::new(ctx, "rawConstantIndices"),
                    DenseI32ArrayAttribute::new(ctx, raw_indices).into(),
                ),
                (
                    Identifier::new(ctx, "elem_type"),
                    TypeAttribute::new(elem_ty).into(),
                ),
            ])
            .add_operands(&operands)
            .add_results(&[result_ty])
            .build()
            .map_err(|e| cctx.emit_internal(format!("GEP: {e}")))
            .ok()?;
        Some(self.append_op(op))
    }

    fn gep_i8(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        base: Value<'c, 'c>,
        idx: Value<'c, 'c>,
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>> {
        let ctx = cctx.mlir;
        self.gep_typed_result(
            cctx,
            base,
            IntegerType::new(ctx, 8).into(),
            &[idx],
            &[i32::MIN],
            ctx.llvm_ptr(),
            loc,
        )
    }

    fn load_typed(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        ptr: Value<'c, 'c>,
        elem_ty: Type<'c>,
        loc: Location<'c>,
    ) -> Option<Value<'c, 'c>> {
        let ctx = cctx.mlir;
        let op = OperationBuilder::new("llvm.load", loc)
            .add_attributes(&[(
                Identifier::new(ctx, "res"),
                TypeAttribute::new(elem_ty).into(),
            )])
            .add_operands(&[ptr])
            .add_results(&[elem_ty])
            .build()
            .map_err(|e| cctx.emit_internal(format!("llvm.load: {e}")))
            .ok()?;
        Some(self.append_op(op))
    }

    fn store_typed(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        ptr: Value<'c, 'c>,
        val: Value<'c, 'c>,
        loc: Location<'c>,
    ) -> Option<()> {
        let op = OperationBuilder::new("llvm.store", loc)
            .add_operands(&[val, ptr])
            .build()
            .map_err(|e| cctx.emit_internal(format!("llvm.store: {e}")))
            .ok()?;
        self.append_operation(op);
        Some(())
    }
}

/// Comparison predicate constants.
pub struct Predicates;
impl Predicates {
    pub const EQ: u64 = CmpiPredicate::Eq as u64;
    pub const NE: u64 = CmpiPredicate::Ne as u64;
    pub const SLT: u64 = CmpiPredicate::Slt as u64;
    pub const SGT: u64 = CmpiPredicate::Sgt as u64;
    pub const SLE: u64 = CmpiPredicate::Sle as u64;
    pub const SGE: u64 = CmpiPredicate::Sge as u64;
    pub const ULT: u64 = CmpiPredicate::Ult as u64;
    pub const UGT: u64 = CmpiPredicate::Ugt as u64;
    pub const ULE: u64 = CmpiPredicate::Ule as u64;
    pub const UGE: u64 = CmpiPredicate::Uge as u64;
}
