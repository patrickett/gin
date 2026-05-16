mod asm;
mod binary;
mod bind;
mod fn_call;
mod format_string;
mod if_;
mod layout;
mod literal;
mod loop_;
mod loop_for;
mod loop_while;
mod tag_call;
mod ty_mapping;
mod when;

pub use ty_mapping::ty_to_mlir;

/// Build an MLIR module from a `TypedFileAst` using the ExprId-based codegen path.
///
/// This creates proper `func.func` operations wrapping the lowered expressions.
/// For each top-level def in the typed AST, a function is created with the def's
/// name, and the lowered expression becomes the function body.
pub fn build_module_from_typed_ast<'a, 'c>(
    context: &'c Context,
    typed: &'a ast::typed::TypedFileAst,
    source: &'a str,
    filename: &str,
) -> Option<melior::ir::Module<'c>> {
    use crate::prelude::*;
    use melior::dialect::func;
    use melior::ir::{Block, Region};
    use std::collections::HashMap;

    let empty: HashMap<_, _> = HashMap::new();
    let sym_table = CompileTimeSymbolTable::new();
    let ctx = crate::lower::CodegenContext::new(
        context,
        Some(typed),
        &empty,
        &sym_table,
        source,
        filename,
        &typed.span_table,
    );

    let module = melior::ir::Module::new(context.unknown_loc());

    for (def_id, bind) in &typed.defs {
        let mut symtab = crate::ScopedSymbolTable::new();
        let func_name = def_id.0.as_str();
        let loc = context.unknown_loc();

        // Create function body region and entry block.
        let region = Region::new();
        let block = Block::new(&[]);
        region.append_block(block);
        let blk = region.first_block().unwrap();

        // Lower the body expression.
        let result_val = match &bind.body {
            ast::typed::BindBody::Expr(eid) => {
                crate::lower::lower_typed_expr(&ctx, *eid, &blk, &mut symtab)
            }
            ast::typed::BindBody::Body { exprs, ret } => {
                for eid in exprs {
                    crate::lower::lower_typed_expr(&ctx, *eid, &blk, &mut symtab);
                }
                ret.and_then(|ret_id| {
                    crate::lower::lower_typed_expr(&ctx, ret_id, &blk, &mut symtab)
                })
            }
            ast::typed::BindBody::Extern => None,
        };

        // Emit return with the proper value and compute the function type.
        let ret_types: Vec<melior::ir::Type<'c>> = match &result_val {
            Some(val) => {
                let ret_ty = val.r#type();
                blk.append_operation(func::r#return(&[*val], loc));
                vec![ret_ty]
            }
            None => {
                blk.append_operation(func::r#return(&[], loc));
                vec![]
            }
        };

        // Create the func.func operation with the correct function type.
        let sym_name = Identifier::new(context, "sym_name");
        let func_type_id = Identifier::new(context, "function_type");
        let func_type = melior::ir::r#type::FunctionType::new(context, &[], &ret_types);
        let func_op = OperationBuilder::new("func.func", loc)
            .add_attributes(&[
                (sym_name, StringAttribute::new(context, func_name).into()),
                (func_type_id, TypeAttribute::new(func_type.into()).into()),
            ])
            .add_regions([region])
            .build()
            .ok()?;
        module.body().append_operation(func_op);
    }

    Some(module)
}

use crate::lower::layout::ty_byte_size_static;
use crate::prelude::*;
use ::span::{SpanId, SpanTable};
use ast::ty::Ty;
use ast::{
    Bind, BindValue, DeclareValue, Expr, FileAst, SymbolTable as CompileTimeSymbolTable, TypeExpr,
    Typed, TypedFileAst, type_surface_mangle_name,
};
use ast::{LocalTypes, TyInfer, TyInferEnv};
use diagnostic::codegen::CodegenSymptom;
use diagnostic::{Diagnostic, DiagnosticCode, DiagnosticLike, TypeSymptom};
use internment::Intern;

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
};

// ScopedSymbolTable is defined in `crate::ScopedSymbolTable` (the struct in lib.rs).
use i256::I256;

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
    /// Typed AST — carries tag_types, fn_return_types, variant_map, and ExprId-based traversal.
    pub typed_ast: Option<&'a TypedFileAst>,
    pub type_info: &'a HashMap<Intern<String>, TypeInfo>,
    pub symbol_table: &'a CompileTimeSymbolTable,
    /// Intern<String>-keyed tag types (converted from TagId-keyed TypedFileAst on construction).
    pub tag_types: HashMap<Intern<String>, Ty>,
    /// Intern<String>-keyed fn return types (converted from DefId-keyed TypedFileAst on construction).
    pub fn_return_types: HashMap<Intern<String>, Ty>,
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
        typed_ast: Option<&'a TypedFileAst>,
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
            tag_types,
            fn_return_types,
            type_info,
            symbol_table,
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

    pub fn lookup_tag(&self, name: Intern<String>) -> Option<&Ty> {
        self.typed_ast
            .and_then(|typed| typed.tag_types.get(&ast::typed::TagId(name)))
    }

    pub fn lookup_variant(&self, name: Intern<String>) -> Option<ast::VariantLookupResult<'_>> {
        self.typed_ast
            .and_then(|typed| typed.variant_map.get(&name))
            .and_then(|candidates| candidates.first())
            .map(|(union, idx, fields)| (*union, *idx, fields.as_slice()))
    }

    pub fn fn_return_ty(&self, name: &Intern<String>) -> Option<&Ty> {
        self.typed_ast
            .and_then(|typed| typed.fn_return_types.get(&ast::typed::DefId(*name)))
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
        ast::is_type_surface(e).then(|| ast::resolve_type_expr_from_map(e, &self.tag_types))
    }

    pub fn param_types<'b>(&self, bind: &'b ast::Bind) -> Vec<(&'b Intern<String>, Ty)> {
        match bind.params().as_ref() {
            None => vec![],
            Some(params) => {
                let subst = bind
                    .receiver_type_surface()
                    .map(|sp| ast::typevars_from_receiver(&sp.value))
                    .unwrap_or_default();
                params
                    .iter()
                    .map(|(name, kind)| {
                        let ty = ast::resolve_parameter_kind_with_subst(
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

/// This is the ExprId-based codegen path. It reads from `ctx.typed_ast` (the typed
/// expression arena) instead of traversing `Box<Typed<Expr>>` pointers.
pub fn lower_typed_expr<'c>(
    ctx: &CodegenContext<'_, 'c>,
    expr_id: ast::typed::ExprId,
    block: &BlockRef<'c, 'c>,
    symtab: &mut ScopedSymbolTable<'c>,
) -> Option<Value<'c, 'c>> {
    let typed_ast = ctx.typed_ast?;
    let expr_ref = typed_ast.expr(expr_id)?;

    match expr_ref.kind {
        ast::typed::TypedExprKind::Lit(lit) => lit.lower(ctx, block, symtab),
        ast::typed::TypedExprKind::FnCall { target, args } => {
            // Lower arguments first.
            let lowered_args: Vec<Value<'c, 'c>> = args
                .as_ref()
                .map(|a| {
                    a.iter()
                        .filter_map(|arg_id| lower_typed_expr(ctx, *arg_id, block, symtab))
                        .collect()
                })
                .unwrap_or_default();

            let fn_name = target.0.as_str();
            let loc = ctx.location();

            // Look up the var in the symtab if no args (variable reference).
            if (args.is_none() || args.as_ref().is_none_or(|a| a.is_empty()))
                && let Some(val) = symtab.get(fn_name)
            {
                return Some(val);
            }

            // Determine return type from the typed AST.
            let return_ty = typed_ast
                .fn_return_types
                .get(target)
                .map(|t| ty_to_mlir(t, ctx.mlir))
                .unwrap_or_else(|| ctx.mlir.i64());

            Some(block.call(ctx.mlir, fn_name, &lowered_args, return_ty, loc))
        }
        ast::typed::TypedExprKind::Bind { name, body } => {
            let val = lower_typed_expr(ctx, *body, block, symtab)?;
            symtab.insert(name.as_str().to_string(), val);
            // Track variable type in var_types for field access.
            ctx.var_types
                .borrow_mut()
                .insert(*name, expr_ref.ty.clone());
            Some(val)
        }
        ast::typed::TypedExprKind::Binary { op, lhs, rhs } => {
            let lhs_val = lower_typed_expr(ctx, *lhs, block, symtab)?;
            let rhs_val = lower_typed_expr(ctx, *rhs, block, symtab)?;
            let loc = ctx.location();

            use melior::ir::operation::OperationBuilder;

            let bin_op_name = match op {
                BinOp::Add => "arith.addi",
                BinOp::Subtract => "arith.subi",
                BinOp::Multiply => "arith.muli",
                _ => return None,
            };

            let result_ty = lhs_val.r#type();
            let op = OperationBuilder::new(bin_op_name, loc)
                .add_operands(&[lhs_val, rhs_val])
                .add_results(&[result_ty])
                .build()
                .ok()?;
            Some(block.append_op(op))
        }
        ast::typed::TypedExprKind::TagCall {
            variant_id,
            discriminant,
            args,
        } => {
            // Lower tag calls using the existing tag_call lowering logic.
            let lowered_args: Vec<Value<'c, 'c>> = args
                .as_ref()
                .map(|a| {
                    a.iter()
                        .filter_map(|arg_id| lower_typed_expr(ctx, *arg_id, block, symtab))
                        .collect()
                })
                .unwrap_or_default();
            tag_call::lower_typed_tag_call(
                ctx,
                &variant_id.union.0,
                &variant_id.name,
                *discriminant,
                &lowered_args,
                block,
                symtab,
            )
        }
        ast::typed::TypedExprKind::SelfRef { target } => {
            // Look up `self` in the symbol table.
            symtab.get(target.0.as_str())
        }
        ast::typed::TypedExprKind::Range { start, end } => {
            let _s = lower_typed_expr(ctx, *start, block, symtab)?;
            let _e = lower_typed_expr(ctx, *end, block, symtab)?;
            None // Range lowering not yet implemented
        }
        ast::typed::TypedExprKind::TupleLit(items) => {
            let vals: Vec<Value<'c, 'c>> = items
                .iter()
                .filter_map(|id| lower_typed_expr(ctx, *id, block, symtab))
                .collect();
            vals.first().copied()
        }
        ast::typed::TypedExprKind::Cast { expr, .. } => lower_typed_expr(ctx, *expr, block, symtab),
        ast::typed::TypedExprKind::TupleAlloc { init, .. }
        | ast::typed::TypedExprKind::TakePtr(init)
        | ast::typed::TypedExprKind::TakeRef(init)
        | ast::typed::TypedExprKind::Negate(init)
        | ast::typed::TypedExprKind::MutArg(init)
        | ast::typed::TypedExprKind::OwnArg(init) => lower_typed_expr(ctx, *init, block, symtab),
        ast::typed::TypedExprKind::TupleGet { base, .. }
        | ast::typed::TypedExprKind::Deref(base) => lower_typed_expr(ctx, *base, block, symtab),
        ast::typed::TypedExprKind::TupleSet { base, value, .. } => {
            lower_typed_expr(ctx, *base, block, symtab)?;
            lower_typed_expr(ctx, *value, block, symtab)
        }
        ast::typed::TypedExprKind::BufGet { buf, index }
        | ast::typed::TypedExprKind::BufSet { buf, index, .. } => {
            lower_typed_expr(ctx, *buf, block, symtab)?;
            lower_typed_expr(ctx, *index, block, symtab)?;
            None
        }
        // Complex types not yet lowered via typed path.
        ast::typed::TypedExprKind::When(_)
        | ast::typed::TypedExprKind::If(_)
        | ast::typed::TypedExprKind::Loop(_)
        | ast::typed::TypedExprKind::FormatString(_)
        | ast::typed::TypedExprKind::List(_)
        | ast::typed::TypedExprKind::Asm(_) => None,
    }
}

pub trait Lower<'c> {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>>;
}

/// This is used for native compilation where we need control over the context.
pub fn build_module_with_context<'c>(
    context: &'c Context,
    ast: &mut FileAst,
    typed_ast: Option<&TypedFileAst>,
    source: &str,
    filename: &str,
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
    let type_info = extract_type_info(ast).unwrap_or_default();

    // If no typed AST provided, transform the file first.
    let owned_typed;
    let typed_ref = match typed_ast {
        Some(t) => t,
        None => {
            owned_typed = ast::typed::transform_file(ast.clone(), ast::typed::FileId(0));
            &owned_typed
        }
    };
    let ctx = CodegenContext::new(
        context,
        Some(typed_ref),
        &type_info,
        &symbol_table,
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
            .expect("result 0 should exist on append_operation")
            .into(),
    )
}

/// Emit `llvm.mlir.global` for a top-level `:=` bind whose value is a `TupleLit`.
/// Returns the global op AND registers the element type in `ctx.global_const_elems`.
fn emit_tuple_lit_global<'c>(
    context: &'c Context,
    ctx: &CodegenContext<'_, 'c>,
    name: &str,
    elems: &[Typed<Expr>],
) -> Option<Operation<'c>> {
    let loc = context.unknown_loc();

    let region = Region::new();
    let init_block = Block::new(&[]);
    region.append_block(init_block);
    let blk = region.first_block().unwrap();

    let mut symtab: ScopedSymbolTable<'c> = ScopedSymbolTable::new();
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
        let elem_ty = first_elem.infer_ty(&ctx.infer_env(&locals));
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
    let elem_ty = init.infer_ty(&ctx.infer_env(&locals));
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

impl<'c> Lower<'c> for Typed<Expr> {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
    ) -> Option<Value<'c, 'c>> {
        ctx.current_span.set(self.span_id);
        self.value.lower(ctx, block, symtab)
    }
}

impl<'c> Lower<'c> for Expr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut ScopedSymbolTable<'c>,
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
            Expr::SelfRef(span) => symtab.get("self").or_else(|| {
                ctx.current_span.set(*span);
                ctx.emit_symptom(TypeSymptom::SelfOutsideMethod);
                None
            }),
            Expr::TagCall(tc) => tc.lower(ctx, block, symtab),
            Expr::AnonymousTag(tag_name, _) => {
                // Bare capitalized tag — treat as a unit variant constructor.
                // Note: unknown tag diagnostics are emitted by typeck; codegen just fails gracefully.
                let (union_name, discriminant, _) = ctx.lookup_variant(*tag_name)?;
                let union_mlir_ty = ctx
                    .lookup_tag(union_name)
                    .map(|ty| ty_to_mlir(ty, ctx.mlir))
                    .unwrap_or_else(|| ctx.mlir.union_type());
                let disc_val = block.const_i64(ctx.mlir, discriminant as i64);
                let undef = block.append_op(ctx.mlir.llvm_undef(union_mlir_ty));
                Some(block.append_op(ctx.mlir.llvm_insertvalue(undef, disc_val, 0)))
            }
            Expr::Asm(asm_expr) => asm_expr.lower(ctx, block, symtab),
            Expr::TupleLit(elems) | Expr::List(elems) => {
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
                    .value
                    .infer_ty(&ctx.infer_env(&*ctx.var_types.borrow()));
                let dst_ty = ctx.lookup_tag(*ty).cloned().unwrap_or(Ty::Int {
                    width: 64,
                    signed: true,
                    value: None,
                });
                // TODO: Handle ConstUnion → base type coercion (e.g. LogLevel → Str).
                // This requires emitting a global string table that maps each
                // ConstUnion discriminant to a Str struct {ptr, len}, then
                // GEP + load at the discriminant offset. See `create_string_global`.
                lower_cast(ctx, block, val, &src_ty, &dst_ty)
            }
            Expr::TakePtr(inner) | Expr::TakeRef(inner) => {
                lower_take_ptr(ctx, block, symtab, inner)
            }
            Expr::Deref(inner) => {
                let ptr = inner.lower(ctx, block, symtab)?;
                let pointee_ty = match inner
                    .value
                    .infer_ty(&ctx.infer_env(&*ctx.var_types.borrow()))
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
            Expr::MutArg(inner) | Expr::OwnArg(inner) => inner.lower(ctx, block, symtab),
            Expr::Negate(inner) => {
                let val = inner.lower(ctx, block, symtab)?;
                let loc = ctx.location();
                let ty = inner.infer_ty(&ctx.infer_env(&*ctx.var_types.borrow()));
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
            Expr::TypeNominal(..) | Expr::TypeQualified(_) | Expr::TypeGeneric { .. } => None,
        }
    }
}

/// Emit an integer constant whose width matches the variant count:
/// - 2 variants       → i1
/// - 3..256 variants  → i8
/// - >256 variants    → i64
pub(crate) fn emit_discriminant_constant<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    disc: i64,
    variant_count: usize,
) -> Value<'c, 'c> {
    if variant_count == 2 {
        let i1_attr = IntegerAttribute::new(IntegerType::new(ctx.mlir, 1).into(), disc).into();
        block.append_op(ctx.mlir.const_op(i1_attr, ctx.mlir.i1()))
    } else if variant_count <= 256 {
        let i8_ty = IntegerType::new(ctx.mlir, 8).into();
        let i8_attr = IntegerAttribute::new(i8_ty, disc).into();
        block.append_op(ctx.mlir.const_op(i8_attr, i8_ty))
    } else {
        block.const_i64(ctx.mlir, disc)
    }
}

/// Extend a small integer (i1 or i8) to i64 for comparison.
/// - 2 variants  → zero-extend (arith.extui)
/// - 3..256      → sign-extend (arith.extsi)
/// - >256        → already i64, no-op
pub(crate) fn emit_discriminant_extend<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    subject: Value<'c, 'c>,
    variant_count: usize,
) -> Value<'c, 'c> {
    let loc = ctx.location();
    if variant_count == 2 {
        let extend_op = match OperationBuilder::new("arith.extui", loc)
            .add_operands(&[subject])
            .add_results(&[ctx.mlir.i64()])
            .build()
        {
            Ok(op) => op,
            Err(e) => {
                ctx.emit_internal(format!("extui build failed: {e}"));
                return subject;
            }
        };
        block.append_op(extend_op)
    } else if variant_count <= 256 {
        let extend_op = match OperationBuilder::new("arith.extsi", loc)
            .add_operands(&[subject])
            .add_results(&[ctx.mlir.i64()])
            .build()
        {
            Ok(op) => op,
            Err(e) => {
                ctx.emit_internal(format!("extsi build failed: {e}"));
                return subject;
            }
        };
        block.append_op(extend_op)
    } else {
        subject
    }
}

/// Shared between `if_` pattern conditions and `when` pattern arms.
pub(crate) fn bind_pattern_payload_fields<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    pattern: &TypeExpr,
    subject_val: Value<'c, 'c>,
    symtab: &mut ScopedSymbolTable<'c>,
) {
    if let TypeExpr::Generic { params, .. } = pattern {
        let variant_name = Intern::<String>::from_ref(type_surface_mangle_name(pattern));
        let payload_fields = ctx
            .lookup_variant(variant_name)
            .map(|(_, _, f)| f)
            .unwrap_or(&[]);
        for (slot, (param_name, _)) in params.iter().enumerate() {
            if param_name.as_str() == "_" {
                continue;
            }
            let field_mlir_ty = payload_fields
                .get(slot)
                .map(|(_, ty)| ty_to_mlir(ty, ctx.mlir))
                .unwrap_or_else(|| ctx.mlir.i64());
            let extracted = block.append_op(ctx.mlir.llvm_extractvalue(
                subject_val,
                (slot + 1) as i64,
                field_mlir_ty,
            ));
            symtab.insert(param_name.as_str().to_string(), extracted);
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
                    .value
                    .as_ref()
                    .map(|sp| spanned_uses_self(sp))
                    .unwrap_or(false)
        }
        BindValue::Extern => false,
    }
}

fn spanned_uses_self(sp: &Typed<Expr>) -> bool {
    expr_uses_self(&sp.value)
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
        Expr::TupleLit(elems) | Expr::List(elems) => elems.iter().any(spanned_uses_self),
        Expr::TupleAlloc { init, .. } => spanned_uses_self(init),
        Expr::TupleGet { base, .. } => spanned_uses_self(base),
        Expr::TupleSet { base, value, .. } => spanned_uses_self(base) || spanned_uses_self(value),
        Expr::BufGet { buf, index } => spanned_uses_self(buf) || spanned_uses_self(index),
        Expr::BufSet { buf, index, value } => {
            spanned_uses_self(buf) || spanned_uses_self(index) || spanned_uses_self(value)
        }
        Expr::TakePtr(e)
        | Expr::TakeRef(e)
        | Expr::Deref(e)
        | Expr::Negate(e)
        | Expr::MutArg(e)
        | Expr::OwnArg(e) => spanned_uses_self(e),
        Expr::Cast { expr: e, .. } => spanned_uses_self(e),
        Expr::Bind(b) => bind_value_uses_self(b.value()),
        Expr::When(w) => {
            w.subject
                .as_ref()
                .map(|s| spanned_uses_self(s))
                .unwrap_or(false)
                || w.arms.iter().any(|arm| match arm {
                    ::ast::WhenArm::Cond {
                        condition, body, ..
                    } => spanned_uses_self(condition) || spanned_uses_self(body),
                    ::ast::WhenArm::Is { body, .. } => spanned_uses_self(body),
                    ::ast::WhenArm::Else(body, _) => spanned_uses_self(body),
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
            ::ast::FormatPart::Expr(e, _) => spanned_uses_self(e),
            _ => false,
        }),
        // Inline asm operand expressions: a use of `self` here is unusual but
        // not rejected. Conservatively scan the operand strings (none today
        // hold expressions; this is a placeholder for future expansion).
        Expr::Asm(_) => false,
        _ => false,
    }
}

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
    let param_info_ref = ctx.param_types(bind);
    let mut param_info: Vec<(Intern<String>, Ty)> =
        param_info_ref.into_iter().map(|(n, t)| (*n, t)).collect();
    if let Some(sp) = bind.receiver_type_surface()
        && bind_value_uses_self(bind.value())
        && let Some(self_ty) = ctx.resolve_type_surface(&sp.value)
    {
        param_info.insert(0, (Intern::<String>::from_ref("self"), self_ty));
    }

    let input_types: Vec<Type<'c>> = param_info
        .iter()
        .map(|(_, ty)| ty_to_mlir(ty, ctx.mlir))
        .collect();

    let env = TyInferEnv {
        tag_types: &ctx.tag_types,
        fn_return_types: &ctx.fn_return_types,
        locals: &HashMap::new(),
        tag_params: None,
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

        let mut symtab: ScopedSymbolTable<'c> = ScopedSymbolTable::new();
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
    symtab: &mut ScopedSymbolTable<'c>,
    inner: &Typed<Expr>,
) -> Option<Value<'c, 'c>> {
    // For a bare variable reference, check if it already lives in a mutable slot.
    if let Expr::FnCall(call) = &inner.value
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
            if let Some(ptr) = symtab.get(name.as_str()) {
                return Some(ptr);
            }
        }
    }
    // Otherwise evaluate the inner expression and spill to a fresh alloca.
    let val = inner.value.lower(ctx, block, symtab)?;
    let elem_ty = inner.infer_ty(&ctx.infer_env(&*ctx.var_types.borrow()));
    let mlir_ty = ty_to_mlir(&elem_ty, ctx.mlir);
    let loc = ctx.location();
    let ptr = block.alloca_typed(ctx.mlir, mlir_ty, loc);
    block.store_typed(ctx, ptr, val, loc)?;
    Some(ptr)
}

/// Falls back to `Ty::Int { width: 8, signed: false }` (byte) if the type cannot be determined.
fn elem_ty_of_array_expr(base: &Typed<Expr>, ctx: &CodegenContext) -> Ty {
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
    symtab: &mut ScopedSymbolTable<'c>,
    init: &Typed<Expr>,
    size: usize,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();

    // Infer element type from init expression.
    let elem_ty = init.infer_ty(&ctx.infer_env(&HashMap::new()));
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
    symtab: &mut ScopedSymbolTable<'c>,
    base: &Typed<Expr>,
    index: usize,
) -> Option<Value<'c, 'c>> {
    let loc = ctx.location();
    let base_val = base.lower(ctx, block, symtab)?;

    // If the base is a struct value (not a pointer), use extractvalue.
    if base_val.r#type() != ctx.mlir.llvm_ptr() {
        let base_ty = base.infer_ty(&ctx.infer_env(&*ctx.var_types.borrow()));
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
    symtab: &mut ScopedSymbolTable<'c>,
    base: &Typed<Expr>,
    index: usize,
    value: &Typed<Expr>,
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

/// Emit destructor calls (`drop` methods) for `#[lin]` variables that are still alive
/// at function exit. This runs after all body expressions but before the return value.
fn emit_lin_destructors<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    symtab: &ScopedSymbolTable<'c>,
) -> Option<()> {
    // Lin types and flow analysis are no longer available via the old Analysis struct.
    // This function is only reachable from the old (non-typed) codegen path which
    // is being deprecated. Return early with no destructors for now.
    let _ = (ctx, block, symtab);
    Some(())

    /*
    let final_ctx = &ctx.analysis.flow.final_context;
    let lin_types = &ctx.analysis.lin_types;
    let mut visited = HashSet::new();

    for (var_name, ty) in ctx.var_types.borrow().iter() {
        if !is_lin_type(ty, lin_types, &mut visited) {
            visited.clear();
            continue;
        }
        visited.clear();

        // Check if this var is still alive at function exit
        if matches!(final_ctx.get_var_state(var_name), Some(VarState::Alive)) {
            // Build the drop function name: TypeName.drop
            // Resolve the type name from the ty
            let type_name = match ty {
                Ty::Record { name, .. } | Ty::Union { name, .. } => name.as_str().to_string(),
                _ => continue,
            };
            let drop_name = format!("{type_name}.drop");

            // Look up the variable's value in the symtab
            let var_value = symtab.get(var_name.as_str())?;

            // Emit a call to the drop function via func.call
            let loc = ctx.location();
            let callee = FlatSymbolRefAttribute::new(ctx.mlir, &drop_name);
            let op = OperationBuilder::new("func.call", loc)
                .add_attributes(&[(Identifier::new(ctx.mlir, "callee"), callee.into())])
                .add_operands(&[var_value])
                .build()
                .ok()?;
            block.append_operation(op);
        }
    }
    Some(())
    */
}

fn lower_bind_value<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    bind_value: &BindValue,
    symtab: &ScopedSymbolTable<'c>,
) -> Option<Option<Value<'c, 'c>>> {
    match bind_value {
        BindValue::Expr(expr) => {
            emit_lin_destructors(ctx, block, symtab)?;
            let val = expr.lower(ctx, block, &mut symtab.clone())?;
            Some(Some(val))
        }
        BindValue::Body { exprs, ret } => {
            let mut local_symtab = symtab.clone();
            for expr in exprs {
                expr.lower(ctx, block, &mut local_symtab)?;
            }
            emit_lin_destructors(ctx, block, &local_symtab)?;
            match &ret.value {
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
