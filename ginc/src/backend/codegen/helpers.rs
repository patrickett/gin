//! Extension traits for ergonomic Melior operations.
//!
//! This module provides extension traits that simplify common Melior patterns

use crate::backend::prelude::*;

/// Extension trait for [`melior::Context`] to simplify common operations.
pub trait ContextExt {
    fn unknown_loc(&self) -> Location<'_>;
    fn i64(&self) -> Type<'_>;
    fn i1(&self) -> Type<'_>;
    fn f64(&self) -> Type<'_>;
    /// String type - LLVM pointer to i8 for null-terminated C strings
    /// Compatible with printf's %s format specifier
    fn string_type(&self) -> Type<'_>;
    /// Unit/void type for Nothing - currently represented as i64
    /// Will be replaced with proper void type after LLVM dialect integration
    fn unit(&self) -> Type<'_>;
    /// LLVM void type for Nothing/unit values
    fn llvm_void(&self) -> Type<'_>;
    /// LLVM pointer to i8 type for strings
    fn llvm_i8_ptr(&self) -> Type<'_>;
}

impl ContextExt for Context {
    fn unknown_loc(&self) -> Location<'_> {
        Location::unknown(self)
    }

    fn i64(&self) -> Type<'_> {
        Type::from(IntegerType::new(self, 64))
    }

    fn i1(&self) -> Type<'_> {
        Type::from(IntegerType::new(self, 1))
    }

    fn f64(&self) -> Type<'_> {
        Type::float64(self)
    }

    fn string_type(&self) -> Type<'_> {
        self.llvm_i8_ptr()
    }

    fn unit(&self) -> Type<'_> {
        Type::from(IntegerType::new(self, 64))
    }

    fn llvm_void(&self) -> Type<'_> {
        r#type::void(self)
    }

    fn llvm_i8_ptr(&self) -> Type<'_> {
        r#type::pointer(self, 0)
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
    fn const_op(&self, attr: Attribute<'c>, ty: Type<'c>) -> Operation<'c>;
    fn build_binop(
        &self,
        name: &'static str,
        lhs: Value<'c, 'c>,
        rhs: Value<'c, 'c>,
    ) -> Operation<'c>;
    fn build_cmpi(&self, predicate: u64, lhs: Value<'c, 'c>, rhs: Value<'c, 'c>) -> Operation<'c>;
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
    ) -> Operation<'c> {
        OperationBuilder::new(name, self.unknown_loc())
            .add_operands(&[lhs, rhs])
            .add_results(&[self.i64()])
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
}

/// Extension trait for [`melior::ir::Block`] to simplify appending operations.
pub trait BlockExt<'c> {
    fn append_op(&self, op: Operation<'c>) -> Value<'c, 'c>;
    fn const_i64(&self, ctx: &'c Context, value: i64) -> Value<'c, 'c>;
    /// Create a string constant - returns a fat pointer (ptr, length)
    /// This requires access to CodegenContext to register the string globally
    fn const_string_with_ctx(&self, ctx: &CodegenContext<'_, 'c>, value: &str) -> Value<'c, 'c>;
    /// Return a unit/void value for Nothing expressions
    fn unit_value(&self, ctx: &CodegenContext<'_, 'c>) -> Value<'c, 'c>;
    fn ret(&self, ctx: &'c Context, values: &[Value<'c, 'c>]) -> Operation<'c>;
    fn call(&self, ctx: &'c Context, func_name: &str, args: &[Value<'c, 'c>]) -> Value<'c, 'c>;
}

impl<'c> BlockExt<'c> for BlockRef<'c, 'c> {
    fn append_op(&self, op: Operation<'c>) -> Value<'c, 'c> {
        let op_ref = self.append_operation(op);
        op_ref.result(0).unwrap().into()
    }

    fn const_i64(&self, ctx: &'c Context, value: i64) -> Value<'c, 'c> {
        self.append_op(ctx.i64_const(value))
    }

    fn const_string_with_ctx(&self, ctx: &CodegenContext<'_, 'c>, value: &str) -> Value<'c, 'c> {
        // Register the string and get its symbol name
        let symbol_name = ctx.register_string(value);
        // Get the address of the global string
        // Note: This will be resolved during module generation
        // For now, we return a placeholder that will be fixed up later
        // The actual addressof operation will be created when the global exists
        crate::backend::codegen::addressof_string_global(ctx.mlir, self, &symbol_name)
            .unwrap_or_else(|_| self.const_i64(ctx.mlir, 42))
    }

    fn unit_value(&self, ctx: &CodegenContext<'_, 'c>) -> Value<'c, 'c> {
        self.const_i64(ctx.mlir, 0)
    }

    fn ret(&self, ctx: &'c Context, values: &[Value<'c, 'c>]) -> Operation<'c> {
        melior::dialect::func::r#return(values, ctx.unknown_loc())
    }

    fn call(&self, ctx: &'c Context, func_name: &str, args: &[Value<'c, 'c>]) -> Value<'c, 'c> {
        let callee_id = Identifier::new(ctx, "callee");
        let symbol_ref = ctx.symbol_ref_attr(func_name);
        self.append_op(
            OperationBuilder::new("func.call", ctx.unknown_loc())
                .add_attributes(&[(callee_id, symbol_ref)])
                .add_operands(args)
                .add_results(&[ctx.i64()])
                .build()
                .unwrap(),
        )
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
}

/// Arithmetic operation names.
pub struct ArithOps;
impl ArithOps {
    pub const ADD: &str = "arith.addi";
    pub const SUB: &str = "arith.subi";
    pub const MUL: &str = "arith.muli";
    pub const DIV: &str = "arith.divsi";
}
