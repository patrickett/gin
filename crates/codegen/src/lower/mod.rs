mod asm;
mod binary;
mod bind;
mod fn_call;
mod format_string;
mod if_;
mod literal;
mod loop_;
mod loop_for;
mod loop_while;
mod tag_call;
mod ty_mapping;
mod when;

pub use ty_mapping::ty_to_mlir;

use crate::prelude::*;
use ::ast::{
    Bind, BindValue, DeclareValue, Expr, FileAst, Spanned, SymbolTable as CompileTimeSymbolTable,
};
use ::span::{SpanId, SpanTable};
use diagnostic::codegen::CodegenSymptom;
use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticLike, TypeSymptom};
use internment::Intern;
use typeck::{LocalTypes, Ty, TyEnv, TyInfer, TyInferEnv, ty_byte_size_static};

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
};

/// Runtime symbol table for MLIR values during codegen.
/// This is separate from the compile-time SymbolTable which tracks metadata.
use i256::I256;

pub type RuntimeSymbolTable<'c> = HashMap<String, Value<'c, 'c>>;

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

fn compute_line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

/// 1-based line and column for MLIR `FileLineCol` locations.
fn byte_offset_to_line_col(line_starts: &[usize], byte: usize) -> (usize, usize) {
    if line_starts.is_empty() {
        return (1, 1);
    }
    let line_idx = line_starts.partition_point(|&s| s <= byte);
    let line_idx = line_idx
        .saturating_sub(1)
        .min(line_starts.len().saturating_sub(1));
    let line_start = line_starts[line_idx];
    let line_no = line_idx + 1;
    let col = byte.saturating_sub(line_start) + 1;
    (line_no, col)
}

pub struct CodegenContext<'a, 'c> {
    pub mlir: &'c Context,
    pub type_info: &'a HashMap<Intern<String>, TypeInfo>,
    pub symbol_table: &'a CompileTimeSymbolTable,
    pub ty_env: &'a TyEnv,
    pub string_literals: RefCell<Vec<String>>,
    pub string_symbols: RefCell<HashMap<String, String>>,
    pub string_counter: Cell<usize>,
    /// Maps variable name → its resolved Ty, used for field-access lowering.
    pub var_types: RefCell<HashMap<Intern<String>, Ty>>,
    /// Names of mutable (`:`) local variables — their symtab value is an alloca ptr.
    /// Cleared at the start of each top-level function lower.
    pub mutable_slots: RefCell<HashSet<String>>,
    /// Element type of global constant arrays (top-level `:=` TupleLit binds), keyed by name.
    pub global_const_elems: RefCell<HashMap<String, Ty>>,
    symptoms: RefCell<Vec<Diagnostic>>,
    pub current_span: Cell<SpanId>,
    pub source_filename: String,
    pub source: &'a str,
    pub span_table: &'a SpanTable,
    pub line_starts: Vec<usize>,
}

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub fn new(
        mlir: &'c Context,
        type_info: &'a HashMap<Intern<String>, TypeInfo>,
        symbol_table: &'a CompileTimeSymbolTable,
        ty_env: &'a TyEnv,
        source: &'a str,
        filename: &str,
        span_table: &'a SpanTable,
    ) -> Self {
        Self {
            mlir,
            type_info,
            symbol_table,
            ty_env,
            string_literals: RefCell::new(Vec::new()),
            string_symbols: RefCell::new(HashMap::new()),
            string_counter: Cell::new(0),
            var_types: RefCell::new(HashMap::new()),
            mutable_slots: RefCell::new(HashSet::new()),
            global_const_elems: RefCell::new(HashMap::new()),
            symptoms: RefCell::new(Vec::new()),
            current_span: Cell::new(SpanId::INVALID),
            source_filename: filename.to_string(),
            source,
            span_table,
            line_starts: compute_line_starts(source),
        }
    }

    pub fn location(&self) -> Location<'c> {
        let id = self.current_span.get();
        if !id.is_valid() {
            return self.mlir.unknown_loc();
        }
        let span = self.span_table.get(id);
        let byte = span.start.min(self.source.len());
        let (line, col) = byte_offset_to_line_col(&self.line_starts, byte);
        Location::new(self.mlir, &self.source_filename, line, col)
    }

    pub fn register_string(&self, s: &str) -> String {
        {
            let symbols = self.string_symbols.borrow();
            if let Some(existing) = symbols.get(s) {
                return existing.clone();
            }
        }

        let counter = self.string_counter.get();
        let name = format!("__string_{}", counter);
        self.string_counter.set(counter + 1);

        let mut symbols = self.string_symbols.borrow_mut();
        let mut literals = self.string_literals.borrow_mut();

        symbols.insert(s.to_string(), name.clone());
        literals.push(s.to_string());

        name
    }

    pub fn emit_symptom<S: DiagnosticLike + Into<DiagnosticCode>>(&self, symptom: S) {
        self.symptoms
            .borrow_mut()
            .push(symptom.into_diagnostic(self.current_span.get()));
    }

    pub fn emit_internal(&self, message: impl Into<String>) {
        self.symptoms.borrow_mut().push(
            CodegenSymptom::Internal {
                message: message.into(),
            }
            .into_diagnostic(self.current_span.get()),
        );
    }

    pub fn drain_symptoms(&self) -> Vec<Diagnostic> {
        self.symptoms.borrow_mut().drain(..).collect()
    }
}

impl LocalTypes for CodegenContext<'_, '_> {
    fn get_type(&self, name: &Intern<String>) -> Option<Ty> {
        self.var_types.borrow().get(name).cloned()
    }
}

pub trait Lower<'c> {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>>;
}

/// Build an MLIR module from the AST with a provided context.
/// This is used for native compilation where we need control over the context.
pub fn build_module_with_context<'c>(
    context: &'c Context,
    ast: &FileAst,
    source: &str,
    filename: &str,
    ty_env: &TyEnv,
) -> (Option<Module<'c>>, Vec<Diagnostic>) {
    // Register dialects
    melior::dialect::DialectHandle::llvm().register_dialect(context);
    context.get_or_load_dialect("arith");
    context.get_or_load_dialect("func");
    context.get_or_load_dialect("scf");
    context.get_or_load_dialect("llvm");

    // Build compile-time symbol table from AST
    let source_path = std::path::PathBuf::new();
    let symbol_table = CompileTimeSymbolTable::from_file(ast, source_path.to_path_buf());
    let type_info = match extract_type_info(ast) {
        Some(info) => info,
        None => {
            return (None, Vec::new());
        }
    };
    let ctx = CodegenContext::new(
        context,
        &type_info,
        &symbol_table,
        ty_env,
        source,
        filename,
        ast.span_table(),
    );

    let module = Module::new(context.unknown_loc());

    // Emit global arrays for top-level `:=` TupleLit and TupleAlloc binds.
    let mut global_ops = Vec::new();
    for (def_name, bind) in &ast.defs {
        if !bind.is_const {
            continue;
        }
        if let BindValue::Expr(boxed) = bind.value() {
            let inner: &Expr = boxed;
            match inner {
                Expr::TupleLit(elems) => {
                    let global_op = emit_tuple_lit_global(context, &ctx, def_name.as_str(), elems);
                    if let Some(op) = global_op {
                        global_ops.push(op);
                    }
                }
                Expr::TupleAlloc { init, size } => {
                    let global_op =
                        emit_tuple_alloc_global(context, &ctx, def_name.as_str(), init, *size);
                    if let Some(op) = global_op {
                        global_ops.push(op);
                    }
                }
                _ => {}
            }
        }
    }

    let mut func_ops = Vec::new();
    for (def_name, bind) in &ast.defs {
        // TODO: flaw diagnostic when a referenced symbol has no matching platform declaration
        if !bind.attributes().matches_current_platform() {
            continue;
        }
        // Skip global TupleLit/TupleAlloc constants — already emitted as globals above.
        if bind.is_const
            && let BindValue::Expr(boxed) = bind.value()
        {
            let inner: &Expr = boxed;
            if matches!(inner, Expr::TupleLit(_) | Expr::TupleAlloc { .. }) {
                continue;
            }
        }
        let func_op = lower_function(&ctx, def_name, bind);
        if let Some(op) = func_op {
            func_ops.push(op);
        }
    }

    // Create string globals (must appear before function ops in the module)
    let string_symbols = ctx.string_symbols.borrow().clone();

    for global_op in global_ops {
        module.body().append_operation(global_op);
    }

    for (value, symbol) in &string_symbols {
        let global_op = create_string_global(context, symbol, value);
        if let Some(op) = global_op {
            module.body().append_operation(op);
        }
    }

    for func_op in func_ops {
        module.body().append_operation(func_op);
    }

    let symptoms = ctx.drain_symptoms();
    (Some(module), symptoms)
}

/// Create a global string constant operation using LLVM dialect.
/// Produces: `llvm.mlir.global internal constant @name("value\00") : !llvm.array<N x i8>`
pub fn create_string_global<'c>(
    context: &'c Context,
    name: &str,
    value: &str,
) -> Option<Operation<'c>> {
    let loc = context.unknown_loc();

    // Null-terminated string bytes
    let with_nul = format!("{}\0", value);
    let byte_len: u32 = match with_nul.len().try_into() {
        Ok(len) => len,
        Err(_) => return None,
    };

    let i8_type = Type::from(IntegerType::new(context, 8));
    let array_type = r#type::array(i8_type, byte_len);

    let linkage_attr = melior::dialect::llvm::attributes::linkage(
        context,
        melior::dialect::llvm::attributes::Linkage::Internal,
    );

    let global = OperationBuilder::new("llvm.mlir.global", loc)
        .add_attributes(&[
            (Identifier::new(context, "sym_name"), context.str_attr(name)),
            (
                Identifier::new(context, "global_type"),
                TypeAttribute::new(array_type).into(),
            ),
            (
                Identifier::new(context, "value"),
                StringAttribute::new(context, &with_nul).into(),
            ),
            (Identifier::new(context, "linkage"), linkage_attr),
            (
                Identifier::new(context, "constant"),
                Attribute::unit(context),
            ),
            (
                Identifier::new(context, "addr_space"),
                IntegerAttribute::new(IntegerType::new(context, 32).into(), 0).into(),
            ),
        ])
        // llvm.mlir.global requires exactly one region in the textual format,
        // even when the value is provided as an attribute.
        .add_regions([Region::new()])
        .build()
        .ok()?;

    Some(global)
}

/// Get the address of a global string using llvm.addressof operation.
/// This returns a pointer to the global that can be used in function calls.
pub fn addressof_string_global<'c>(
    context: &'c Context,
    block: &BlockRef<'c, 'c>,
    global_name: &str,
) -> Option<Value<'c, 'c>> {
    let loc = context.unknown_loc();
    let global_name_id = Identifier::new(context, "global_name");
    let symbol_ref = context.symbol_ref_attr(global_name);

    let addressof_op = OperationBuilder::new("llvm.mlir.addressof", loc)
        .add_attributes(&[(global_name_id, symbol_ref)])
        .add_results(&[context.llvm_ptr()])
        .build()
        .ok()?;

    // Append the operation to the block and return the result
    Some(
        block
            .append_operation(addressof_op)
            .result(0)
            .unwrap()
            .into(),
    )
}

/// Emit `llvm.mlir.global` for a top-level `:=` bind whose value is a `TupleLit`.
/// Returns the global op AND registers the element type in `ctx.global_const_elems`.
fn emit_tuple_lit_global<'c>(
    context: &'c Context,
    ctx: &CodegenContext<'_, 'c>,
    name: &str,
    elems: &[Spanned<Expr>],
) -> Option<Operation<'c>> {
    let loc = context.unknown_loc();

    let region = Region::new();
    let init_block = Block::new(&[]);
    region.append_block(init_block);
    let blk = region.first_block().unwrap();

    let mut symtab: RuntimeSymbolTable<'c> = HashMap::new();
    let elem_vals: Vec<Value<'c, 'c>> = elems
        .iter()
        .map(|e| e.lower(ctx, &blk, &mut symtab))
        .collect::<Option<Vec<_>>>()?;

    let elem_mlir_ty = elem_vals
        .first()
        .map(|v| v.r#type())
        .unwrap_or_else(|| context.i64());
    let n = elem_vals.len() as u32;
    let array_mlir_ty = r#type::array(elem_mlir_ty, n);

    let locals: HashMap<Intern<String>, Ty> = HashMap::new();
    if let Some(first_elem) = elems.first() {
        let elem_ty = first_elem.infer_ty(&ctx.ty_env.infer_env(&locals));
        ctx.global_const_elems
            .borrow_mut()
            .insert(name.to_string(), elem_ty);
    }

    let undef = blk.append_op(ctx.mlir.llvm_undef(array_mlir_ty));
    let mut current = undef;
    for (i, val) in elem_vals.iter().enumerate() {
        let pos = DenseI64ArrayAttribute::new(context, &[i as i64]);
        let insert_op = OperationBuilder::new("llvm.insertvalue", loc)
            .add_attributes(&[(Identifier::new(context, "position"), pos.into())])
            .add_operands(&[current, *val])
            .enable_result_type_inference()
            .build()
            .ok()?;
        current = blk.append_op(insert_op);
    }

    let ret_op = OperationBuilder::new("llvm.return", loc)
        .add_operands(&[current])
        .build()
        .ok()?;
    blk.append_operation(ret_op);

    let linkage_attr = melior::dialect::llvm::attributes::linkage(
        context,
        melior::dialect::llvm::attributes::Linkage::Internal,
    );

    let global = OperationBuilder::new("llvm.mlir.global", loc)
        .add_attributes(&[
            (Identifier::new(context, "sym_name"), context.str_attr(name)),
            (
                Identifier::new(context, "global_type"),
                TypeAttribute::new(array_mlir_ty).into(),
            ),
            (Identifier::new(context, "linkage"), linkage_attr),
            (
                Identifier::new(context, "constant"),
                Attribute::unit(context),
            ),
            (
                Identifier::new(context, "addr_space"),
                IntegerAttribute::new(IntegerType::new(context, 32).into(), 0).into(),
            ),
        ])
        .add_regions([region])
        .build()
        .ok()?;

    Some(global)
}

/// Emit `llvm.mlir.global` for a top-level `:=` bind whose value is a `TupleAlloc`.
/// Emits a mutable zero-initialized global array and registers the element type in
/// `ctx.global_const_elems`.
fn emit_tuple_alloc_global<'c>(
    context: &'c Context,
    ctx: &CodegenContext<'_, 'c>,
    name: &str,
    init: &Expr,
    size: usize,
) -> Option<Operation<'c>> {
    let loc = context.unknown_loc();

    let locals: HashMap<Intern<String>, Ty> = HashMap::new();
    let elem_ty = init.infer_ty(&ctx.ty_env.infer_env(&locals));
    let elem_mlir_ty = ty_to_mlir(&elem_ty, context);
    let array_mlir_ty = r#type::array(elem_mlir_ty, size as u32);

    ctx.global_const_elems
        .borrow_mut()
        .insert(name.to_string(), elem_ty);

    let region = Region::new();
    let init_block = Block::new(&[]);
    region.append_block(init_block);
    let blk = region.first_block().unwrap();

    let zero_op = OperationBuilder::new("llvm.mlir.zero", loc)
        .add_results(&[array_mlir_ty])
        .build()
        .ok()?;
    let zero_val = blk.append_op(zero_op);

    let ret_op = OperationBuilder::new("llvm.return", loc)
        .add_operands(&[zero_val])
        .build()
        .ok()?;
    blk.append_operation(ret_op);

    let linkage_attr = melior::dialect::llvm::attributes::linkage(
        context,
        melior::dialect::llvm::attributes::Linkage::Internal,
    );

    let global = OperationBuilder::new("llvm.mlir.global", loc)
        .add_attributes(&[
            (Identifier::new(context, "sym_name"), context.str_attr(name)),
            (
                Identifier::new(context, "global_type"),
                TypeAttribute::new(array_mlir_ty).into(),
            ),
            (Identifier::new(context, "linkage"), linkage_attr),
            (
                Identifier::new(context, "addr_space"),
                IntegerAttribute::new(IntegerType::new(context, 32).into(), 0).into(),
            ),
        ])
        .add_regions([region])
        .build()
        .ok()?;

    Some(global)
}

fn extract_type_info(ast: &FileAst) -> Option<HashMap<Intern<String>, TypeInfo>> {
    let mut type_info = HashMap::new();

    for (tag_name, documented) in &ast.tags {
        if let DeclareValue::Range(min, max) = documented.value() {
            type_info.insert(
                *tag_name,
                TypeInfo {
                    min: *min,
                    max: *max,
                },
            );
        }
    }

    Some(type_info)
}

// === Expression lowering ===

impl<'c> Lower<'c> for Spanned<Expr> {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        ctx.current_span.set(self.1);
        self.0.lower(ctx, block, symtab)
    }
}

impl<'c> Lower<'c> for Expr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        match self {
            Expr::Lit(lit) => lit.lower(ctx, block, symtab),
            Expr::Binary(bin) => bin.lower(ctx, block, symtab),
            Expr::FnCall(call) => call.lower(ctx, block, symtab),
            Expr::Bind(bind) => bind.lower(ctx, block, symtab),
            Expr::Loop(loop_expr) => loop_expr.lower(ctx, block, symtab),
            Expr::When(when_expr) => when_expr.lower(ctx, block, symtab),
            Expr::If(if_expr) => if_expr.lower(ctx, block, symtab),
            Expr::FormatString(fs) => fs.lower(ctx, block, symtab),
            Expr::Range(_) => {
                ctx.emit_internal(
                    "Range lowering not yet implemented (only valid inside a for-in)",
                );
                None
            }
            Expr::SelfRef(span) => symtab.get("self").copied().or_else(|| {
                ctx.current_span.set(*span);
                ctx.emit_symptom(TypeSymptom::SelfOutsideMethod);
                None
            }),
            Expr::TagCall(tc) => tc.lower(ctx, block, symtab),
            Expr::AnonymousTag(tag_name, _) => {
                // Bare capitalized tag — treat as a unit variant constructor.
                // Note: unknown tag diagnostics are emitted by typeck; codegen just fails gracefully.
                let (union_name, discriminant, _) = ctx.ty_env.lookup_variant(*tag_name)?;
                let union_mlir_ty = ctx
                    .ty_env
                    .lookup_tag(union_name)
                    .map(|ty| ty_to_mlir(ty, ctx.mlir))
                    .unwrap_or_else(|| ctx.mlir.union_type());
                let disc_val = block.const_i64(ctx.mlir, discriminant as i64);
                let undef = block.append_op(ctx.mlir.llvm_undef(union_mlir_ty));
                Some(block.append_op(ctx.mlir.llvm_insertvalue(undef, disc_val, 0)))
            }
            Expr::Asm(asm_expr) => asm_expr.lower(ctx, block, symtab),
            Expr::TupleLit(elems) => {
                let elem_vals: Vec<Value<'c, 'c>> = elems
                    .iter()
                    .map(|e| e.lower(ctx, block, symtab))
                    .collect::<Option<Vec<_>>>()?;
                let field_types: Vec<Type<'c>> = elem_vals.iter().map(|v| v.r#type()).collect();
                let struct_ty = r#type::r#struct(ctx.mlir, &field_types, false);
                let mut val = block.append_op(ctx.mlir.llvm_undef(struct_ty));
                for (i, field_val) in elem_vals.iter().enumerate() {
                    val = block.append_op(ctx.mlir.llvm_insertvalue(val, *field_val, i as i64));
                }
                Some(val)
            }
            Expr::TupleAlloc { init, size } => lower_tuple_alloc(ctx, block, symtab, init, *size),
            Expr::TupleGet { base, index } => lower_tuple_get(ctx, block, symtab, base, *index),
            Expr::TupleSet { base, index, value } => {
                lower_tuple_set(ctx, block, symtab, base, *index, value)
            }
            Expr::BufGet { buf, index } => {
                let loc = ctx.location();
                let ptr = buf.lower(ctx, block, symtab)?;
                let idx = index.lower(ctx, block, symtab)?;
                let elem_ty = elem_ty_of_array_expr(buf, ctx);
                let elem_bytes = ty_byte_size_static(&elem_ty) as i64;
                let byte_idx = if elem_bytes == 1 {
                    idx
                } else {
                    let stride = block.const_i64(ctx.mlir, elem_bytes);
                    block.append_op(ctx.mlir.build_binop(
                        ArithOps::MUL,
                        idx,
                        stride,
                        ctx.mlir.i64(),
                    ))
                };
                let elem_ptr = block.gep_i8(ctx, ptr, byte_idx, loc)?;
                let elem_mlir_ty = ty_to_mlir(&elem_ty, ctx.mlir);
                Some(block.load_typed(ctx, elem_ptr, elem_mlir_ty, loc)?)
            }
            Expr::BufSet { buf, index, value } => {
                let loc = ctx.location();
                let ptr = buf.lower(ctx, block, symtab)?;
                let idx = index.lower(ctx, block, symtab)?;
                let val = value.lower(ctx, block, symtab)?;
                let elem_ty = elem_ty_of_array_expr(buf, ctx);
                let elem_bytes = ty_byte_size_static(&elem_ty) as i64;
                let byte_idx = if elem_bytes == 1 {
                    idx
                } else {
                    let stride = block.const_i64(ctx.mlir, elem_bytes);
                    block.append_op(ctx.mlir.build_binop(
                        ArithOps::MUL,
                        idx,
                        stride,
                        ctx.mlir.i64(),
                    ))
                };
                let elem_ptr = block.gep_i8(ctx, ptr, byte_idx, loc)?;
                block.store_typed(ctx, elem_ptr, val, loc)?;
                Some(block.const_i64(ctx.mlir, 0))
            }
            Expr::Cast { expr, ty } => {
                let val = expr.lower(ctx, block, symtab)?;
                let src_ty = expr
                    .0
                    .infer_ty(&ctx.ty_env.infer_env(&*ctx.var_types.borrow()));
                let dst_ty = ctx.ty_env.lookup_tag(*ty).cloned().unwrap_or(Ty::Int {
                    width: 64,
                    signed: true,
                    value: None,
                });
                lower_cast(ctx, block, val, &src_ty, &dst_ty)
            }
            Expr::TakePtr(inner) | Expr::TakeRef(inner) => {
                lower_take_ptr(ctx, block, symtab, inner)
            }
            Expr::Deref(inner) => {
                let ptr = inner.lower(ctx, block, symtab)?;
                let pointee_ty = match inner
                    .0
                    .infer_ty(&ctx.ty_env.infer_env(&*ctx.var_types.borrow()))
                {
                    Ty::Ptr { inner } | Ty::Ref { inner } => *inner,
                    _ => Ty::Int {
                        width: 64,
                        signed: true,
                        value: None,
                    },
                };
                let mlir_ty = ty_to_mlir(&pointee_ty, ctx.mlir);
                let loc = ctx.location();
                Some(block.load_typed(ctx, ptr, mlir_ty, loc)?)
            }
            Expr::Negate(inner) => {
                let val = inner.lower(ctx, block, symtab)?;
                let loc = ctx.location();
                let ty = inner.infer_ty(&ctx.ty_env.infer_env(&*ctx.var_types.borrow()));
                if ty.is_float() {
                    let neg_op = OperationBuilder::new("arith.negf", loc)
                        .add_operands(&[val])
                        .add_results(&[ctx.mlir.f64()])
                        .build()
                        .ok()?;
                    Some(block.append_op(neg_op))
                } else {
                    let val_ty = val.r#type();
                    let zero = block.const_int(ctx.mlir, val_ty, 0);
                    Some(block.append_op(ctx.mlir.build_binop(ArithOps::SUB, zero, val, val_ty)))
                }
            }
            Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => {
                ctx.emit_internal(
                    "type/pattern syntax may only appear in pattern or type positions, not as a value",
                );
                None
            }
        }
    }
}

/// True if any expression reachable from `bind_value` references `self`.
///
/// Used to decide whether a method-like bind (one with a `receiver_type`)
/// should take an implicit `self` parameter at codegen time. Constructors
/// like `Range.new(start x, end x) Range[x]: (start, end)` have a receiver
/// type for namespacing/dispatch but don't use `self`, so they should be
/// emitted as static functions taking only their declared params.
fn bind_value_uses_self(value: &BindValue) -> bool {
    match value {
        BindValue::Expr(e) => spanned_uses_self(e),
        BindValue::Body { exprs, ret } => {
            exprs.iter().any(spanned_uses_self)
                || ret
                    .0
                    .as_ref()
                    .map(|sp| spanned_uses_self(sp))
                    .unwrap_or(false)
        }
        BindValue::Extern => false,
    }
}

fn spanned_uses_self(sp: &Spanned<Expr>) -> bool {
    expr_uses_self(&sp.0)
}

fn expr_uses_self(expr: &Expr) -> bool {
    match expr {
        Expr::SelfRef(_) => true,
        Expr::Binary(b) => spanned_uses_self(&b.lhs) || spanned_uses_self(&b.rhs),
        Expr::FnCall(c) => {
            // `self.x` and `self.method(args)` start with `self` as the path root.
            if c.path.root.as_str() == "self" {
                return true;
            }
            c.args
                .as_ref()
                .map(|a| a.iter().any(spanned_uses_self))
                .unwrap_or(false)
        }
        Expr::TagCall(tc) => tc.args.iter().any(spanned_uses_self),
        Expr::TupleLit(elems) => elems.iter().any(spanned_uses_self),
        Expr::TupleAlloc { init, .. } => spanned_uses_self(init),
        Expr::TupleGet { base, .. } => spanned_uses_self(base),
        Expr::TupleSet { base, value, .. } => spanned_uses_self(base) || spanned_uses_self(value),
        Expr::BufGet { buf, index } => spanned_uses_self(buf) || spanned_uses_self(index),
        Expr::BufSet { buf, index, value } => {
            spanned_uses_self(buf) || spanned_uses_self(index) || spanned_uses_self(value)
        }
        Expr::TakePtr(e) | Expr::TakeRef(e) | Expr::Deref(e) | Expr::Negate(e) => {
            spanned_uses_self(e)
        }
        Expr::Cast { expr: e, .. } => spanned_uses_self(e),
        Expr::Bind(b) => bind_value_uses_self(b.value()),
        Expr::When(w) => {
            w.subject
                .as_ref()
                .map(|s| spanned_uses_self(s))
                .unwrap_or(false)
                || w.arms.iter().any(|arm| match arm {
                    ::ast::WhenArm::Cond { condition, body } => {
                        spanned_uses_self(condition) || spanned_uses_self(body)
                    }
                    ::ast::WhenArm::Is { body, .. } => spanned_uses_self(body),
                    ::ast::WhenArm::Else(body) => spanned_uses_self(body),
                })
        }
        Expr::If(if_expr) => {
            let cond_uses_self = match &if_expr.condition {
                ::ast::IfCondition::Bool(c) => spanned_uses_self(c),
                ::ast::IfCondition::Pattern { subject, .. } => spanned_uses_self(subject),
            };
            cond_uses_self || if_expr.body.iter().any(spanned_uses_self)
        }
        Expr::Loop(loop_expr) => match loop_expr {
            ::ast::Loop::ForIn(f) => {
                spanned_uses_self(&f.iter) || f.exprs.iter().any(spanned_uses_self)
            }
            ::ast::Loop::While(w) => {
                spanned_uses_self(&w.cond) || w.exprs.iter().any(spanned_uses_self)
            }
        },
        Expr::FormatString(fs) => fs.parts.iter().any(|p| match p {
            ::ast::FormatPart::Expr(e) => spanned_uses_self(e),
            _ => false,
        }),
        // Inline asm operand expressions: a use of `self` here is unusual but
        // not rejected. Conservatively scan the operand strings (none today
        // hold expressions; this is a placeholder for future expansion).
        Expr::Asm(_) => false,
        _ => false,
    }
}

/// Lower a function definition to MLIR func.func operation.
pub fn lower_function<'c>(
    ctx: &CodegenContext<'_, 'c>,
    def_name: &Intern<String>,
    bind: &Bind,
) -> Option<Operation<'c>> {
    let name = def_name.as_str();
    let loc = ctx.location();

    // Build owned param list, prepending `self` for instance methods only.
    //
    // A bind on a type acts as either:
    //   - an instance method (uses `self` in the body) — prepended `self` arg
    //     is required (e.g. `Bool.to_string Str := when self then ...`).
    //   - a static / associated function (e.g. constructor `Range.new`) —
    //     does not reference `self`; should not take an implicit self.
    //
    // Detect by scanning the body for `Expr::SelfRef`. This keeps both call
    // shapes working without changing the surface syntax: `b.to_string` and
    // `Range.new(12, 1200)` both just dispatch to the mangled name.
    let param_info_ref = ctx.ty_env.param_types(bind);
    let mut param_info: Vec<(Intern<String>, Ty)> =
        param_info_ref.into_iter().map(|(n, t)| (*n, t)).collect();
    if let Some(sp) = bind.receiver_type_surface()
        && bind_value_uses_self(bind.value())
        && let Some(self_ty) = ctx.ty_env.resolve_type_surface(&sp.0)
    {
        param_info.insert(0, (Intern::<String>::from_ref("self"), self_ty));
    }

    let input_types: Vec<Type<'c>> = param_info
        .iter()
        .map(|(_, ty)| ty_to_mlir(ty, ctx.mlir))
        .collect();

    let env = TyInferEnv {
        tag_types: &ctx.ty_env.tag_types,
        fn_return_types: &ctx.ty_env.fn_return_types,
        locals: &HashMap::new(),
    };
    let return_ty = bind.infer_ty(&env);
    let ret_types: Vec<Type<'c>> = match &return_ty {
        Ty::Unit => vec![],
        ty => vec![ty_to_mlir(ty, ctx.mlir)],
    };
    let func_type = melior::ir::r#type::FunctionType::new(ctx.mlir, &input_types, &ret_types);

    let sym_name = Identifier::new(ctx.mlir, "sym_name");
    let func_type_id = Identifier::new(ctx.mlir, "function_type");

    // Extern declarations: emit a func.func with an empty region and private linkage.
    if bind.value() == &BindValue::Extern {
        let extern_region = Region::new();
        return OperationBuilder::new("func.func", loc)
            .add_attributes(&[
                (sym_name, ctx.mlir.str_attr(name)),
                (func_type_id, ctx.mlir.type_attr(Type::from(func_type))),
                (
                    Identifier::new(ctx.mlir, "sym_visibility"),
                    ctx.mlir.str_attr("private"),
                ),
            ])
            .add_regions([extern_region])
            .build()
            .ok();
    }

    // Clear per-function mutable-slot tracking before lowering the body.
    // TODO: flaw diagnostic — rebinding a module-level (top-level) bind with `:` should
    // be flagged at compile time as an anti-pattern; use `:=` for module constants instead.
    ctx.mutable_slots.borrow_mut().clear();

    let region = Region::new();
    {
        let block_args: Vec<_> = input_types.iter().map(|ty| (*ty, loc)).collect();
        let block = Block::new(&block_args);
        region.append_block(block);
        let block = region.first_block().unwrap();

        let mut symtab: RuntimeSymbolTable<'c> = HashMap::new();
        for (i, (param_name, param_ty)) in param_info.iter().enumerate() {
            let arg = block.argument(i).unwrap();
            symtab.insert(param_name.as_str().to_string(), arg.into());
            ctx.var_types
                .borrow_mut()
                .insert(*param_name, param_ty.clone());
        }

        let result = lower_bind_value(ctx, &block, bind.value(), &symtab)?;

        let ret_loc = ctx.location();
        let ret_op = match result {
            None => block.ret(ctx.mlir, &[], ret_loc),
            Some(v) => {
                if matches!(return_ty, Ty::Unit) {
                    block.ret(ctx.mlir, &[], ret_loc)
                } else {
                    block.ret(ctx.mlir, &[v], ret_loc)
                }
            }
        };
        block.append_operation(ret_op);
    }

    OperationBuilder::new("func.func", loc)
        .add_attributes(&[
            (sym_name, ctx.mlir.str_attr(name)),
            (func_type_id, ctx.mlir.type_attr(Type::from(func_type))),
        ])
        .add_regions([region])
        .build()
        .ok()
}

/// Lower `@expr` / `^expr` — produce a pointer to the value.
///
/// * If the inner expression is a mutable slot (alloca'd variable), return the alloca ptr directly.
/// * Otherwise, spill the SSA value to a fresh alloca and return that ptr.
fn lower_take_ptr<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    symtab: &mut RuntimeSymbolTable<'c>,
    inner: &Spanned<Expr>,
) -> Option<Value<'c, 'c>> {
    // For a bare variable reference, check if it already lives in a mutable slot.
    if let Expr::FnCall(call) = &inner.0
        && call.path.segments.is_empty()
        && call.args.is_none()
    {
        let name = call.path.root;
        if ctx.mutable_slots.borrow().contains(name.as_str()) {
            let var_ty = ctx.var_types.borrow().get(&name).cloned();
            if matches!(
                var_ty,
                Some(Ty::Array { .. }) | Some(Ty::Ptr { .. }) | Some(Ty::Ref { .. })
            ) {
                // For pointer-valued slots (arrays, ptr vars), the user wants the data
                // pointer itself — evaluate normally to load it from the slot.
                return inner.lower(ctx, block, symtab);
            }
            if let Some(&ptr) = symtab.get(name.as_str()) {
                return Some(ptr);
            }
        }
    }
    // Otherwise evaluate the inner expression and spill to a fresh alloca.
    let val = inner.0.lower(ctx, block, symtab)?;
    let elem_ty = inner.infer_ty(&ctx.ty_env.infer_env(&*ctx.var_types.borrow()));
    let mlir_ty = ty_to_mlir(&elem_ty, ctx.mlir);
    let loc = ctx.location();
    let ptr = block.alloca_typed(ctx.mlir, mlir_ty, loc);
    block.store_typed(ctx, ptr, val, loc)?;
    Some(ptr)
}

/// Look up the element type of a base expression that should have `Ty::Array`.
/// Falls back to `Ty::Int { width: 8, signed: false }` (byte) if the type cannot be determined.
fn elem_ty_of_array_expr(base: &Spanned<Expr>, ctx: &CodegenContext) -> Ty {
    let base: &Expr = base;
    if let Expr::FnCall(call) = base
        && call.path.segments.is_empty()
        && call.args.is_none()
    {
        let name = call.path.root;
        if let Some(Ty::Array { elem, .. }) = ctx.var_types.borrow().get(&name).cloned() {
            return *elem;
        }
        if let Some(elem_ty) = ctx.global_const_elems.borrow().get(name.as_str()).cloned() {
            return elem_ty;
        }
    }
    Ty::Int {
        width: 8,
        signed: false,
        value: None,
    }
}

fn lower_tuple_alloc<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    symtab: &mut RuntimeSymbolTable<'c>,
    init: &Spanned<Expr>,
    size: usize,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();

    // Infer element type from init expression.
    let elem_ty = init.infer_ty(&ctx.ty_env.infer_env(&HashMap::new()));
    let elem_bytes = ty_byte_size_static(&elem_ty);
    let total_bytes = size * elem_bytes;

    // Allocate stack buffer. The buffer is uninitialized — callers write before reading.
    let count = block.const_i64(ctx.mlir, total_bytes as i64);
    let ptr = block.append_op(melior::dialect::llvm::alloca(
        ctx.mlir,
        count,
        ctx.mlir.llvm_ptr(),
        loc,
        melior::dialect::llvm::AllocaOptions::default().elem_type(Some(TypeAttribute::new(
            IntegerType::new(ctx.mlir, 8).into(),
        ))),
    ));

    // Lower init expression for side-effects / type inference, but don't use the value.
    // The actual initialization of individual elements is done via TupleSet.
    let _ = init.lower(ctx, block, symtab)?;

    Some(ptr)
}

fn lower_tuple_get<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    symtab: &mut RuntimeSymbolTable<'c>,
    base: &Spanned<Expr>,
    index: usize,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();
    let base_val = base.lower(ctx, block, symtab)?;

    // If the base is a struct value (not a pointer), use extractvalue.
    if base_val.r#type() != ctx.mlir.llvm_ptr() {
        let base_ty = base.infer_ty(&ctx.ty_env.infer_env(&*ctx.var_types.borrow()));
        let field_ty = match base_ty {
            Ty::Tuple(ref fields) => fields
                .get(index)
                .map(|t| ty_to_mlir(t, ctx.mlir))
                .unwrap_or_else(|| ctx.mlir.i64()),
            _ => ctx.mlir.i64(),
        };
        return Some(block.append_op(ctx.mlir.llvm_extractvalue(base_val, index as i64, field_ty)));
    }

    // Existing pointer-based path (for TupleAlloc and global arrays).
    let elem_ty = elem_ty_of_array_expr(base, ctx);
    let elem_bytes = ty_byte_size_static(&elem_ty);
    let elem_mlir_ty = ty_to_mlir(&elem_ty, ctx.mlir);
    let byte_offset = block.const_i64(ctx.mlir, (index * elem_bytes) as i64);
    let elem_ptr = block.gep_i8(ctx, base_val, byte_offset, loc)?;

    let load_op = OperationBuilder::new("llvm.load", loc)
        .add_attributes(&[(
            Identifier::new(ctx.mlir, "res"),
            TypeAttribute::new(elem_mlir_ty).into(),
        )])
        .add_operands(&[elem_ptr])
        .add_results(&[elem_mlir_ty])
        .build()
        .ok()?;
    Some(block.append_op(load_op))
}

fn lower_tuple_set<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    symtab: &mut RuntimeSymbolTable<'c>,
    base: &Spanned<Expr>,
    index: usize,
    value: &Spanned<Expr>,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();
    let ptr = base.lower(ctx, block, symtab)?;

    let elem_ty = elem_ty_of_array_expr(base, ctx);
    let elem_bytes = ty_byte_size_static(&elem_ty);

    let byte_offset = block.const_i64(ctx.mlir, (index * elem_bytes) as i64);
    let elem_ptr = block.gep_i8(ctx, ptr, byte_offset, loc)?;

    let val = value.lower(ctx, block, symtab)?;
    let store_op = OperationBuilder::new("llvm.store", loc)
        .add_operands(&[val, elem_ptr])
        .build()
        .ok()?;
    block.append_operation(store_op);

    Some(block.const_i64(ctx.mlir, 0))
}

fn lower_bind_value<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    bind_value: &BindValue,
    symtab: &RuntimeSymbolTable<'c>,
) -> Option<Option<Value<'c, 'c>>> {
    match bind_value {
        BindValue::Expr(expr) => {
            let val = expr.lower(ctx, block, &mut symtab.clone())?;
            Some(Some(val))
        }
        BindValue::Body { exprs, ret } => {
            let mut local_symtab = symtab.clone();
            for expr in exprs {
                expr.lower(ctx, block, &mut local_symtab)?;
            }
            match &ret.0 {
                Some(expr) => {
                    let val = expr.lower(ctx, block, &mut local_symtab)?;
                    Some(Some(val))
                }
                None => Some(None),
            }
        }
        BindValue::Extern => Some(None),
    }
}

/// Emit the appropriate MLIR cast op between two numeric types.
fn lower_cast<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    val: Value<'c, 'c>,
    src_ty: &Ty,
    dst_ty: &Ty,
) -> Option<Value<'c, 'c>> {
    if src_ty == dst_ty {
        return Some(val);
    }
    let loc = ctx.location();
    let dst_mlir = ty_to_mlir(dst_ty, ctx.mlir);

    // Pointer-to-integer: `ptr as Int` — emits llvm.ptrtoint.
    if (src_ty.is_ptr_or_ref() || matches!(src_ty, Ty::Array { .. })) && dst_ty.is_int() {
        let op = OperationBuilder::new("llvm.ptrtoint", loc)
            .add_operands(&[val])
            .add_results(&[dst_mlir])
            .build()
            .ok()?;
        return Some(block.append_op(op));
    }

    let op_name = match (src_ty, dst_ty) {
        // Narrowing: always truncate (same for signed/unsigned)
        (Ty::Int { width: s, .. }, Ty::Int { width: d, .. }) if s > d => "arith.trunci",
        // Widening signed: sign-extend
        (
            Ty::Int {
                width: s,
                signed: true,
                ..
            },
            Ty::Int { width: d, .. },
        ) if s < d => "arith.extsi",
        // Widening unsigned: zero-extend
        (
            Ty::Int {
                width: s,
                signed: false,
                ..
            },
            Ty::Int { width: d, .. },
        ) if s < d => "arith.extui",
        // Same width, different signedness: no-op (bit pattern is identical)
        (Ty::Int { .. }, Ty::Int { .. }) => return Some(val),
        // Signed int → float
        (Ty::Int { signed: true, .. }, Ty::Float { .. }) => "arith.sitofp",
        // Unsigned int → float
        (Ty::Int { signed: false, .. }, Ty::Float { .. }) => "arith.uitofp",
        // Float → signed int
        (Ty::Float { .. }, Ty::Int { signed: true, .. }) => "arith.fptosi",
        // Float → unsigned int
        (Ty::Float { .. }, Ty::Int { signed: false, .. }) => "arith.fptoui",
        _ => {
            ctx.emit_internal(format!("unsupported cast: {src_ty:?} → {dst_ty:?}"));
            return None;
        }
    };
    let op = OperationBuilder::new(op_name, loc)
        .add_operands(&[val])
        .add_results(&[dst_mlir])
        .build()
        .ok()?;
    Some(block.append_op(op))
}
