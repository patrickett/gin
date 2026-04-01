//! Extension traits for ergonomic Melior operations.
//!
//! This module provides extension traits that simplify common Melior patterns

use crate::prelude::*;

/// Extension trait for [`melior::Context`] to simplify common operations.
pub trait ContextExt {
    fn unknown_loc(&self) -> Location<'_>;
    fn i64(&self) -> Type<'_>;
    fn i128(&self) -> Type<'_>;
    fn i1(&self) -> Type<'_>;
    fn f64(&self) -> Type<'_>;
    /// String type — `!llvm.struct<(ptr, i64)>` fat pointer (data, len)
    fn string_type(&self) -> Type<'_>;
    /// Unit/void type - currently represented as i64
    fn unit(&self) -> Type<'_>;
    /// LLVM void type for unit values
    fn llvm_void(&self) -> Type<'_>;
    /// LLVM opaque pointer type
    fn llvm_ptr(&self) -> Type<'_>;
    /// Tagged union type — `!llvm.struct<(i64, i64)>` (discriminant, payload)
    /// All union values use this uniform layout; unit variants leave the payload slot as zero.
    fn union_type(&self) -> Type<'_>;
}

impl ContextExt for Context {
    fn unknown_loc(&self) -> Location<'_> {
        Location::unknown(self)
    }

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

    fn string_type(&self) -> Type<'_> {
        r#type::r#struct(
            self,
            &[r#type::pointer(self, 0), IntegerType::new(self, 64).into()],
            false,
        )
    }

    fn unit(&self) -> Type<'_> {
        Type::from(IntegerType::new(self, 64))
    }

    fn llvm_void(&self) -> Type<'_> {
        r#type::void(self)
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
    fn i64_const(&self, value: i64) -> Operation<'c>;
    /// Create an integer constant of arbitrary bit-width.
    /// Note: melior's `IntegerAttribute::new` currently takes `i64`, so values
    /// exceeding 64 bits are truncated. Full i128 constant support requires a
    /// future melior API update or manual LLVMAPInt construction.
    fn int_const(&self, ty: Type<'c>, value: i128) -> Operation<'c>;
    fn const_op(&self, attr: Attribute<'c>, ty: Type<'c>) -> Operation<'c>;
    fn build_binop(
        &self,
        name: &'static str,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        result_ty: Type<'c>,
    ) -> Operation<'c>;
    fn build_cmpi(&self, predicate: u64, lhs: Value<'c, 'c>, rhs: Value<'c, 'c>) -> Operation<'c>;
    fn build_cmpf(&self, predicate: u64, lhs: Value<'c, 'c>, rhs: Value<'c, 'c>) -> Operation<'c>;
    /// `llvm.mlir.undef` — produce an undefined value of the given type
    fn llvm_undef(&self, ty: Type<'c>) -> Operation<'c>;
    /// `llvm.insertvalue` — insert a value into an aggregate at `position`
    fn llvm_insertvalue(
        &self,
        container: Value<'c, 'c>,
        value: Value<'c, 'c>,
        position: i64,
    ) -> Operation<'c>;
    /// `llvm.extractvalue` — extract a value from an aggregate at `position`
    fn llvm_extractvalue(
        &self,
        container: Value<'c, 'c>,
        position: i64,
        result_type: Type<'c>,
    ) -> Operation<'c>;
}

impl<'c> OperationBuilderExt<'c> for &'c Context {
    fn i64_const(&self, value: i64) -> Operation<'c> {
        let ty = self.i64();
        let attr = IntegerAttribute::new(ty, value);
        let value_id = Identifier::new(self, "value");
        OperationBuilder::new("arith.constant", self.unknown_loc())
            .add_attributes(&[(value_id, attr.into())])
            .add_results(&[ty])
            .build()
            .unwrap()
    }

    fn int_const(&self, ty: Type<'c>, value: i128) -> Operation<'c> {
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
        OperationBuilder::new("arith.constant", self.unknown_loc())
            .add_attributes(&[(value_id, attr)])
            .add_results(&[ty])
            .build()
            .unwrap()
    }

    fn const_op(&self, attr: Attribute<'c>, ty: Type<'c>) -> Operation<'c> {
        let value_id = Identifier::new(self, "value");
        OperationBuilder::new("arith.constant", self.unknown_loc())
            .add_attributes(&[(value_id, attr)])
            .add_results(&[ty])
            .build()
            .unwrap()
    }

    fn build_binop(
        &self,
        name: &'static str,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
        result_ty: Type<'c>,
    ) -> Operation<'c> {
        OperationBuilder::new(name, self.unknown_loc())
            .add_operands(&[lhs, rhs])
            .add_results(&[result_ty])
            .build()
            .unwrap()
    }

    fn build_cmpi(&self, predicate: u64, lhs: Value<'c, 'c>, rhs: Value<'c, 'c>) -> Operation<'c> {
        let pred_attr = self.i64_attr(predicate as i64);
        let pred_id = Identifier::new(self, "predicate");
        OperationBuilder::new("arith.cmpi", self.unknown_loc())
            .add_attributes(&[(pred_id, pred_attr)])
            .add_operands(&[lhs, rhs])
            .add_results(&[self.i1()])
            .build()
            .unwrap()
    }

    fn build_cmpf(&self, predicate: u64, lhs: Value<'c, 'c>, rhs: Value<'c, 'c>) -> Operation<'c> {
        let pred_attr = self.i64_attr(predicate as i64);
        let pred_id = Identifier::new(self, "predicate");
        OperationBuilder::new("arith.cmpf", self.unknown_loc())
            .add_attributes(&[(pred_id, pred_attr)])
            .add_operands(&[lhs, rhs])
            .add_results(&[self.i1()])
            .build()
            .unwrap()
    }

    fn llvm_undef(&self, ty: Type<'c>) -> Operation<'c> {
        melior::dialect::llvm::undef(ty, self.unknown_loc())
    }

    fn llvm_insertvalue(
        &self,
        container: Value<'c, 'c>,
        value: Value<'c, 'c>,
        position: i64,
    ) -> Operation<'c> {
        melior::dialect::llvm::insert_value(
            self,
            container,
            DenseI64ArrayAttribute::new(self, &[position]),
            value,
            self.unknown_loc(),
        )
    }

    fn llvm_extractvalue(
        &self,
        container: Value<'c, 'c>,
        position: i64,
        result_type: Type<'c>,
    ) -> Operation<'c> {
        melior::dialect::llvm::extract_value(
            self,
            container,
            DenseI64ArrayAttribute::new(self, &[position]),
            result_type,
            self.unknown_loc(),
        )
    }
}

/// Extension trait for [`melior::ir::Block`] to simplify appending operations.
pub trait BlockExt<'c> {
    fn append_op(&self, op: Operation<'c>) -> Value<'c, 'c>;
    fn const_i64(&self, ctx: &'c Context, value: i64) -> Value<'c, 'c>;
    /// Create an integer constant of the given MLIR type.
    fn const_int(&self, ctx: &'c Context, ty: Type<'c>, value: i128) -> Value<'c, 'c>;
    /// Create a string constant - returns a fat pointer (ptr, length)
    /// This requires access to CodegenContext to register the string globally
    fn const_string_with_ctx(&self, ctx: &CodegenContext<'_, 'c>, value: &str) -> Value<'c, 'c>;
    /// Return a unit/void value
    fn unit_value(&self, ctx: &CodegenContext<'_, 'c>) -> Value<'c, 'c>;
    fn ret(&self, ctx: &'c Context, values: &[Value<'c, 'c>]) -> Operation<'c>;
    fn call_void(&self, ctx: &'c Context, func_name: &str, args: &[Value<'c, 'c>]);
    fn call(
        &self,
        ctx: &'c Context,
        func_name: &str,
        args: &[Value<'c, 'c>],
        return_type: Type<'c>,
    ) -> Value<'c, 'c>;
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
        op_ref.result(0).unwrap().into()
    }

    fn const_i64(&self, ctx: &'c Context, value: i64) -> Value<'c, 'c> {
        self.append_op(ctx.i64_const(value))
    }

    fn const_int(&self, ctx: &'c Context, ty: Type<'c>, value: i128) -> Value<'c, 'c> {
        self.append_op(ctx.int_const(ty, value))
    }

    fn const_string_with_ctx(&self, ctx: &CodegenContext<'_, 'c>, value: &str) -> Value<'c, 'c> {
        let c = ctx.mlir;
        let symbol_name = ctx.register_string(value);

        // llvm.mlir.addressof @symbol → !llvm.ptr
        let ptr = crate::addressof_string_global(c, self, &symbol_name)
            .expect("addressof should succeed");

        // llvm.mlir.undef : !llvm.struct<(ptr, i64)>
        let undef = self.append_op(c.llvm_undef(c.string_type()));

        // llvm.insertvalue ptr, undef[0]
        let with_ptr = self.append_op(c.llvm_insertvalue(undef, ptr, 0));

        // arith.constant <byte len> : i64
        let len = self.const_i64(c, value.len() as i64);

        // llvm.insertvalue len, struct[1]
        self.append_op(c.llvm_insertvalue(with_ptr, len, 1))
    }

    fn unit_value(&self, ctx: &CodegenContext<'_, 'c>) -> Value<'c, 'c> {
        self.const_i64(ctx.mlir, 0)
    }

    fn ret(&self, ctx: &'c Context, values: &[Value<'c, 'c>]) -> Operation<'c> {
        melior::dialect::func::r#return(values, ctx.unknown_loc())
    }

    fn call_void(&self, ctx: &'c Context, func_name: &str, args: &[Value<'c, 'c>]) {
        let callee_id = Identifier::new(ctx, "callee");
        let symbol_ref = ctx.symbol_ref_attr(func_name);
        self.append_operation(
            OperationBuilder::new("func.call", ctx.unknown_loc())
                .add_attributes(&[(callee_id, symbol_ref)])
                .add_operands(args)
                .build()
                .unwrap(),
        );
    }

    fn call(
        &self,
        ctx: &'c Context,
        func_name: &str,
        args: &[Value<'c, 'c>],
        return_type: Type<'c>,
    ) -> Value<'c, 'c> {
        let callee_id = Identifier::new(ctx, "callee");
        let symbol_ref = ctx.symbol_ref_attr(func_name);
        self.append_op(
            OperationBuilder::new("func.call", ctx.unknown_loc())
                .add_attributes(&[(callee_id, symbol_ref)])
                .add_operands(args)
                .add_results(&[return_type])
                .build()
                .unwrap(),
        )
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
        let count_one = self.const_i64(ctx, 1);
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

/// Arithmetic operation names.
pub struct ArithOps;
impl ArithOps {
    pub const ADD: &str = "arith.addi";
    pub const SUB: &str = "arith.subi";
    pub const MUL: &str = "arith.muli";
    pub const DIV: &str = "arith.divsi";
    pub const DIVU: &str = "arith.divui";
    pub const REM: &str = "arith.remsi";
    pub const REMU: &str = "arith.remui";
    pub const ADDF: &str = "arith.addf";
    pub const SUBF: &str = "arith.subf";
    pub const MULF: &str = "arith.mulf";
    pub const DIVF: &str = "arith.divf";
    pub const REMF: &str = "arith.remf";
    pub const ANDI: &str = "arith.andi";
    pub const ORI: &str = "arith.ori";
    pub const XORI: &str = "arith.xori";
    pub const SHLI: &str = "arith.shli";
    pub const SHRI: &str = "arith.shrsi";
    pub const SHRUI: &str = "arith.shrui";
}

/// Floating-point comparison predicates for `arith.cmpf`.
pub struct FPredicates;
impl FPredicates {
    pub const OEQ: u64 = 1;
    pub const OGT: u64 = 2;
    pub const OGE: u64 = 3;
    pub const OLT: u64 = 4;
    pub const OLE: u64 = 5;
    pub const ONE: u64 = 6;
}
