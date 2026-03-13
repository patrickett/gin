use crate::codegen::prelude::*;
use crate::diagnostic::codegen::CodegenSymptom;
use crate::ast::{
    Bind, BindValue, DeclareValue, Expr, FileAst, Literal, SymbolTable as CompileTimeSymbolTable,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

/// Runtime symbol table for MLIR values during codegen.
/// This is separate from the compile-time SymbolTable which tracks metadata.
pub type RuntimeSymbolTable<'c> = HashMap<String, Value<'c, 'c>>;

#[derive(Debug, Clone)]
pub struct TypeInfo {
    pub min: i64,
    pub max: i64,
}

impl TypeInfo {
    pub fn bit_width(&self) -> u32 {
        let range = self.max - self.min;
        if range <= u8::MAX as i64 + 1 {
            8
        } else if range <= u16::MAX as i64 + 1 {
            16
        } else if range <= (i32::MAX as i64 + 1) {
            32
        } else {
            64
        }
    }
}

pub struct CodegenContext<'a, 'c> {
    pub mlir: &'c Context,
    pub type_info: &'a HashMap<IStr, TypeInfo>,
    pub symbol_table: &'a CompileTimeSymbolTable,
    pub string_literals: RefCell<Vec<String>>,
    pub string_symbols: RefCell<HashMap<String, String>>,
    pub string_counter: Cell<usize>,
}

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub fn new(
        mlir: &'c Context,
        type_info: &'a HashMap<IStr, TypeInfo>,
        symbol_table: &'a CompileTimeSymbolTable,
    ) -> Self {
        Self {
            mlir,
            type_info,
            symbol_table,
            string_literals: RefCell::new(Vec::new()),
            string_symbols: RefCell::new(HashMap::new()),
            string_counter: Cell::new(0),
        }
    }

    pub fn location(&self) -> Location<'c> {
        self.mlir.unknown_loc()
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
}

pub trait Lower<'c> {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom>;
}

impl<'c> Lower<'c> for Box<Expr> {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        self.as_ref().lower(ctx, block, symtab)
    }
}

pub fn generate_mlir(ast: &FileAst) -> Result<String, CodegenSymptom> {
    let context = Context::new();
    melior::dialect::DialectHandle::llvm().register_dialect(&context);
    context.get_or_load_dialect("arith");
    context.get_or_load_dialect("func");
    context.get_or_load_dialect("scf");
    context.get_or_load_dialect("llvm");

    // Build compile-time symbol table from AST
    // TODO: Track actual source path instead of using empty path
    let source_path = std::path::PathBuf::new();
    let symbol_table = CompileTimeSymbolTable::from_file(ast, source_path.to_path_buf());
    let type_info = extract_type_info(ast)?;
    let ctx = CodegenContext::new(&context, &type_info, &symbol_table);

    let module = Module::new(context.unknown_loc());

    let mut func_ops = Vec::new();
    for (def_name, bind) in &ast.defs {
        let func_op = lower_function(&ctx, def_name, bind)?;
        func_ops.push(func_op);
    }

    // Create string globals (must appear before function ops in the module)
    let string_symbols = ctx.string_symbols.borrow().clone();

    for (value, symbol) in &string_symbols {
        let global_op = create_string_global(&context, symbol, value)?;
        module.body().append_operation(global_op);
    }

    for func_op in func_ops {
        module.body().append_operation(func_op);
    }

    // TODO: return Operation
    Ok(module.as_operation().to_string())
}

/// Build an MLIR module from the AST with a provided context.
/// This is used for native compilation where we need control over the context.
pub fn build_module_with_context<'c>(
    context: &'c Context,
    ast: &FileAst,
) -> Result<Module<'c>, CodegenSymptom> {
    // Register dialects
    melior::dialect::DialectHandle::llvm().register_dialect(context);
    context.get_or_load_dialect("arith");
    context.get_or_load_dialect("func");
    context.get_or_load_dialect("scf");
    context.get_or_load_dialect("llvm");

    // Build compile-time symbol table from AST
    let source_path = std::path::PathBuf::new();
    let symbol_table = CompileTimeSymbolTable::from_file(ast, source_path.to_path_buf());
    let type_info = extract_type_info(ast)?;
    let ctx = CodegenContext::new(context, &type_info, &symbol_table);

    let module = Module::new(context.unknown_loc());

    let mut func_ops = Vec::new();
    for (def_name, bind) in &ast.defs {
        let func_op = lower_function(&ctx, def_name, bind)?;
        func_ops.push(func_op);
    }

    // Create string globals (must appear before function ops in the module)
    let string_symbols = ctx.string_symbols.borrow().clone();

    for (value, symbol) in &string_symbols {
        let global_op = create_string_global(context, symbol, value)?;
        module.body().append_operation(global_op);
    }

    for func_op in func_ops {
        module.body().append_operation(func_op);
    }

    Ok(module)
}

/// Create a global string constant operation using LLVM dialect.
/// Produces: `llvm.mlir.global internal constant @name("value\00") : !llvm.array<N x i8>`
pub fn create_string_global<'c>(
    context: &'c Context,
    name: &str,
    value: &str,
) -> Result<Operation<'c>, CodegenSymptom> {
    let loc = context.unknown_loc();

    // Null-terminated string bytes
    let with_nul = format!("{}\0", value);
    let byte_len: u32 = with_nul
        .len()
        .try_into()
        .map_err(|_| CodegenSymptom::Internal("String too long for u32".to_string()))?;

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
        .build()
        .map_err(|e| CodegenSymptom::Internal(format!("Failed to build string global: {}", e)))?;

    Ok(global)
}

/// Get the address of a global string using llvm.addressof operation.
/// This returns a pointer to the global that can be used in function calls.
pub fn addressof_string_global<'c>(
    context: &'c Context,
    block: &BlockRef<'c, 'c>,
    global_name: &str,
) -> Result<Value<'c, 'c>, CodegenSymptom> {
    let loc = context.unknown_loc();
    let global_name_id = Identifier::new(context, "global_name");
    let symbol_ref = context.symbol_ref_attr(global_name);

    let addressof_op = OperationBuilder::new("llvm.mlir.addressof", loc)
        .add_attributes(&[(global_name_id, symbol_ref)])
        .add_results(&[context.llvm_ptr()])
        .build()
        .map_err(|e| CodegenSymptom::Internal(format!("Failed to build addressof: {}", e)))?;

    // Append the operation to the block and return the result
    Ok(block
        .append_operation(addressof_op)
        .result(0)
        .unwrap()
        .into())
}

fn extract_type_info(ast: &FileAst) -> Result<HashMap<IStr, TypeInfo>, CodegenSymptom> {
    let mut type_info = HashMap::new();

    for (tag_name, documented) in &ast.tags {
        if let DeclareValue::Range(range) = &documented.value() {
            type_info.insert(
                *tag_name,
                TypeInfo {
                    min: range.start,
                    max: range.end,
                },
            );
        }
    }

    Ok(type_info)
}

// === Expression lowering ===

impl<'c> Lower<'c> for Expr {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        match self {
            Expr::Lit(lit) => lit.lower(ctx, block, symtab),
            Expr::Binary(bin) => bin.lower(ctx, block, symtab),
            Expr::FnCall(call) => call.lower(ctx, block, symtab),
            Expr::Bind(bind) => bind.lower(ctx, block, symtab),
            Expr::Loop(_) => Err(CodegenSymptom::Internal(
                "Loop lowering not yet implemented".to_string(),
            )),
            Expr::FormatString(_) => Err(CodegenSymptom::Internal(
                "FormatString lowering not yet implemented".to_string(),
            )),
            Expr::Range(_) => Err(CodegenSymptom::Internal(
                "Range lowering not yet implemented".to_string(),
            )),
            Expr::Nothing => Ok(block.unit_value(ctx)),
        }
    }
}

/// Lower a function definition to MLIR func.func operation.
pub fn lower_function<'c>(
    ctx: &CodegenContext<'_, 'c>,
    def_name: &IStr,
    bind: &Bind,
) -> Result<Operation<'c>, CodegenSymptom> {
    let name = def_name.as_str();
    let loc = ctx.location();

    let (param_names, input_types): (Vec<&IStr>, Vec<Type<'c>>) =
        if let Some(params) = bind.params().as_ref() {
            let names: Vec<&IStr> = params.keys().collect();
            let types: Vec<Type<'c>> = names.iter().map(|_| ctx.mlir.i64()).collect();
            (names, types)
        } else {
            (vec![], vec![])
        };

    let return_type = infer_return_type(ctx, bind)?;
    let func_type = melior::ir::r#type::FunctionType::new(ctx.mlir, &input_types, &[return_type]);

    let region = Region::new();
    {
        let block_args: Vec<_> = input_types.iter().map(|ty| (*ty, loc)).collect();
        let block = Block::new(&block_args);
        region.append_block(block);
        let block = region.first_block().unwrap();

        let mut symtab: RuntimeSymbolTable<'c> = HashMap::new();
        for (i, param_name) in param_names.iter().enumerate() {
            let arg = block.argument(i).unwrap();
            symtab.insert(param_name.as_str().to_string(), arg.into());
        }

        let result = lower_bind_value(ctx, &block, bind.value(), &symtab)?;

        let ret_op = if let Some(result1) = result {
            block.ret(ctx.mlir, &[result1])
        } else {
            block.ret(ctx.mlir, &[])
        };
        block.append_operation(ret_op);
    }

    let sym_name = Identifier::new(ctx.mlir, "sym_name");
    let func_type_id = Identifier::new(ctx.mlir, "function_type");

    OperationBuilder::new("func.func", loc)
        .add_attributes(&[
            (sym_name, ctx.mlir.str_attr(name)),
            (func_type_id, ctx.mlir.type_attr(Type::from(func_type))),
        ])
        .add_regions([region])
        .build()
        .map_err(|e| CodegenSymptom::Internal(format!("Failed to build func: {}", e)))
}

fn lower_bind_value<'c>(
    ctx: &CodegenContext<'_, 'c>,
    block: &BlockRef<'c, 'c>,
    bind_value: &BindValue,
    symtab: &RuntimeSymbolTable<'c>,
) -> Result<Option<Value<'c, 'c>>, CodegenSymptom> {
    match bind_value {
        BindValue::Expr(expr) => Ok(Some(expr.lower(ctx, block, &mut symtab.clone())?)),
        BindValue::Body { exprs, ret } => {
            let mut local_symtab = symtab.clone();
            for expr in exprs {
                expr.lower(ctx, block, &mut local_symtab)?;
            }
            match &ret.0 {
                Some(expr) => Ok(Some(expr.lower(ctx, block, &mut local_symtab)?)),
                None => Ok(None),
            }
        }
    }
}

fn infer_return_type<'c>(
    ctx: &CodegenContext<'_, 'c>,
    bind: &Bind,
) -> Result<Type<'c>, CodegenSymptom> {
    match bind.value() {
        BindValue::Expr(expr) => infer_expr_type(ctx, expr),
        BindValue::Body { ret, .. } => match &ret.0 {
            Some(expr) => infer_expr_type(ctx, expr),
            None => Ok(ctx.mlir.i64()),
        },
    }
}

fn infer_expr_type<'c>(
    ctx: &CodegenContext<'_, 'c>,
    expr: &Expr,
) -> Result<Type<'c>, CodegenSymptom> {
    match expr {
        Expr::Lit(literal) => match literal {
            Literal::Int(_) | Literal::Number(_) => Ok(ctx.mlir.i64()),
            Literal::Float(_) => Ok(ctx.mlir.f64()),
            Literal::String(_) => Ok(ctx.mlir.string_type()),
        },
        _ => Ok(ctx.mlir.i64()), // TODO: proper type inference
    }
}
