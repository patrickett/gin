//! Extension traits for ergonomic Melior operations.
//!
//! This module provides extension traits that simplify common Melior patterns

use crate::prelude::*;

/// Extension trait for [`melior::Context`] to simplify common operations.
pub trait ContextExt {
    fn i64(&self) -> Type<'_>;
    fn i128(&self) -> Type<'_>;
    fn i1(&self) -> Type<'_>;
    fn f64(&self) -> Type<'_>;
    /// LLVM opaque pointer type
    fn llvm_ptr(&self) -> Type<'_>;
    /// Tagged union type — `!llvm.struct<(i64, i64)>` (discriminant, payload)
    /// All union values use this uniform layout; unit variants leave the payload slot as zero.
    fn union_type(&self) -> Type<'_>;
}

impl ContextExt for Context {
    fn i64(&self) -> Type<'_> {
        Type::from(IntegerType::new(self, 64))
    }

    fn i128(&self) -> Type<'_> {
        Type::from(IntegerType::new(self, 128))
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

    fn union_type(&self) -> Type<'_> {
        r#type::r#struct(self, &[self.i64(), self.i64()], false)
    }
}

/// Extension trait for creating MLIR attributes.
pub trait AttributeExt<'c> {
    fn i64_attr(&self, value: i64) -> Attribute<'c>;
    fn f64_attr(&self, value: f64) -> Attribute<'c>;
    fn str_attr(&self, value: &str) -> Attribute<'c>;
    fn symbol_ref_attr(&self, name: &str) -> Attribute<'c>;
    fn type_attr(&self, ty: Type<'c>) -> Attribute<'c>;
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

    fn type_attr(&self, ty: Type<'c>) -> Attribute<'c> {
        TypeAttribute::new(ty).into()
    }
}

/// Extension trait for building MLIR operations.
pub trait OperationBuilderExt<'c> {
    fn i64_const(&self, value: i64, loc: Location<'c>) -> Operation<'c>;
    /// Create an integer constant of arbitrary bit-width.
    /// Note: melior's `IntegerAttribute::new` currently takes `i64`, so values
    /// exceeding 64 bits are truncated. Full i128 constant support requires a
    /// future melior API update or manual LLVMAPInt construction.
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
    fn build_cmpf(
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
        let attr = IntegerAttribute::new(ty, value);
        let value_id = Identifier::new(self, "value");
        OperationBuilder::new("arith.constant", loc)
            .add_attributes(&[(value_id, attr.into())])
            .add_results(&[ty])
            .build()
            .expect("arith.constant build should succeed")
    }

    fn int_const(&self, ty: Type<'c>, value: i128, loc: Location<'c>) -> Operation<'c> {
        let attr: Attribute<'c> = if value >= i64::MIN as i128 && value <= i64::MAX as i128 {
            // Fast path: value fits in i64, use native IntegerAttribute
            IntegerAttribute::new(ty, value as i64).into()
        } else {
            // Slow path: value exceeds i64 range — use MLIR's attribute parser
            // which internally constructs an LLVM APInt from the string form.
            let width = IntegerType::try_from(ty)
                .map(|it| it.width())
                .unwrap_or(128);
            let source = format!("{value} : i{width}");
            Attribute::parse(self, &source).unwrap_or_else(|| {
                // Fallback: truncate if parser fails (should not happen for valid i128)
                IntegerAttribute::new(ty, value as i64).into()
            })
        };
        let value_id = Identifier::new(self, "value");
        OperationBuilder::new("arith.constant", loc)
            .add_attributes(&[(value_id, attr)])
            .add_results(&[ty])
            .build()
            .expect("arith.constant build should succeed")
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

    fn build_cmpf(
        &self,
        predicate: u64,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        loc: Location<'c>,
    ) -> Operation<'c> {
        let pred_attr = self.i64_attr(predicate as i64);
        let pred_id = Identifier::new(self, "predicate");
        OperationBuilder::new("arith.cmpf", loc)
            .add_attributes(&[(pred_id, pred_attr)])
            .add_operands(&[lhs, rhs])
            .add_results(&[self.i1()])
            .build()
            .expect("arith.cmpf build should succeed")
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
    fn const_string_with_ctx(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        value: &str,
        ty: &ast::Ty,
        loc: Location<'c>,
    ) -> Value<'c, 'c>;
    /// Return a unit/void value
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
    /// `llvm.intr.memset` — fill `len` bytes starting at `dst` with `val` (i8).
    fn memset_bytes(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        dst: Value<'c, 'c>,
        val: Value<'c, 'c>,
        len: Value<'c, 'c>,
        loc: Location<'c>,
    ) -> Option<()>;
    /// `llvm.alloca 1 x elem_ty` — allocate a single scalar slot on the stack.
    fn alloca_typed(&self, ctx: &'c Context, elem_ty: Type<'c>, loc: Location<'c>)
    -> Value<'c, 'c>;
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

    fn const_string_with_ctx(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        value: &str,
        ty: &ast::Ty,
        loc: Location<'c>,
    ) -> Value<'c, 'c> {
        let c = ctx.mlir;
        let symbol_name = ctx.register_string(value);

        // llvm.mlir.addressof @symbol → !llvm.ptr
        let ptr = ctx
            .addressof_string_global(self, &symbol_name, loc)
            .expect("addressof should succeed");
        let len = self.const_i64(c, value.len() as i64, loc);

        ctx.build_string_value(self, ptr, len, ty, loc)
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
        let op = OperationBuilder::new("llvm.getelementptr", loc)
            .add_attributes(&[
                (
                    Identifier::new(ctx, "rawConstantIndices"),
                    DenseI32ArrayAttribute::new(ctx, &[i32::MIN]).into(),
                ),
                (
                    Identifier::new(ctx, "elem_type"),
                    TypeAttribute::new(IntegerType::new(ctx, 8).into()).into(),
                ),
            ])
            .add_operands(&[base, idx])
            .add_results(&[ctx.llvm_ptr()])
            .build()
            .map_err(|e| cctx.emit_internal(format!("GEP: {e}")))
            .ok()?;
        Some(self.append_op(op))
    }

    fn memset_bytes(
        &self,
        cctx: &CodegenContext<'_, 'c>,
        dst: Value<'c, 'c>,
        val: Value<'c, 'c>,
        len: Value<'c, 'c>,
        loc: Location<'c>,
    ) -> Option<()> {
        let ctx = cctx.mlir;
        let op = OperationBuilder::new("llvm.intr.memset", loc)
            .add_attributes(&[(
                Identifier::new(ctx, "isVolatile"),
                IntegerAttribute::new(IntegerType::new(ctx, 1).into(), 0).into(),
            )])
            .add_operands(&[dst, val, len])
            .build()
            .map_err(|e| cctx.emit_internal(format!("memset: {e}")))
            .ok()?;
        self.append_operation(op);
        Some(())
    }

    fn alloca_typed(
        &self,
        ctx: &'c Context,
        elem_ty: Type<'c>,
        loc: Location<'c>,
    ) -> Value<'c, 'c> {
        let count_one = self.const_i64(ctx, 1, loc);
        self.append_op(melior::dialect::llvm::alloca(
            ctx,
            count_one,
            r#type::pointer(ctx, 0),
            loc,
            melior::dialect::llvm::AllocaOptions::default()
                .elem_type(Some(TypeAttribute::new(elem_ty))),
        ))
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
    pub const EQ: u64 = 0;
    pub const NE: u64 = 1;
    pub const SLT: u64 = 2;
    pub const SGT: u64 = 3;
    pub const SLE: u64 = 4;
    pub const SGE: u64 = 5;
    pub const ULT: u64 = 6;
    pub const UGT: u64 = 7;
    pub const ULE: u64 = 8;
    pub const UGE: u64 = 9;
}
