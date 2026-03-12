use crate::backend::prelude::*;
use crate::diagnostic::codegen::CodegenSymptom;
use crate::frontend::prelude::{
    DeclareValue, Expr, FileAst, SymbolTable as CompileTimeSymbolTable,
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
