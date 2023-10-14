use std::{
    path::Path,
    process::{Command, Stdio},
};

use inkwell::{
    context::Context,
    targets::{CodeModel, FileType, RelocMode, Target, TargetMachine},
    values::IntValue,
    OptimizationLevel,
};

use crate::parser::{Expr, Literal, Op};

// Recursive function to generate IR for an expression
fn generate_expr_ir<'a>(
    context: &'a Context,
    module: &'a inkwell::module::Module,
    builder: &'a inkwell::builder::Builder,
    expr: &'a Expr,
) -> IntValue<'a> {
    match expr {
        Expr::BinExpr { lhs, op, rhs } => {
            let lhs_value = generate_expr_ir(context, module, builder, lhs);
            let rhs_value = generate_expr_ir(context, module, builder, rhs);

            match op {
                Op::Add => builder.build_int_add(lhs_value, rhs_value, "addtmp"),
                Op::Sub => builder.build_int_sub(lhs_value, rhs_value, "subtmp"),
                Op::Mul => builder.build_int_mul(lhs_value, rhs_value, "multmp"),
                Op::Div => todo!(),
                Op::Assign => todo!(),
            }
        }
        // Expr::Assignment { symbol, expr } => todo!(),
        Expr::Literal(literal) => match literal {
            Literal::String(_) => todo!(),
            Literal::Number(n) => context.i32_type().const_int(*n as u64, false),
        },
        Expr::Id(_) => todo!(),
    }
}

pub fn compile_exprs(exprs: Vec<Expr>, mod_name: &str) {
    let context = Context::create();
    let module = context.create_module("add");
    let builder = context.create_builder();

    // Define the main function
    let main_func = module.add_function("main", context.i32_type().fn_type(&[], false), None);
    let entry_block = context.append_basic_block(main_func, "entry");

    // Set the builder position to the entry block
    builder.position_at_end(entry_block);

    // Generate IR for each expression in the Vec
    let result = generate_expr_ir(&context, &module, &builder, &exprs[0]);

    // Return the result at the end of the main function
    builder.build_return(Some(&result));

    // Print LLVM IR to console
    module.print_to_stderr();

    module
        .print_to_file(format!("{}.ll", mod_name))
        .expect("Failed to write to .ll file");

    // Emit machine code
    Target::initialize_native(&Default::default()).expect("Failed to initialize target");
    let target_triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&target_triple).expect("Failed to get target");
    let target_machine = target
        .create_target_machine(
            &target_triple,
            "generic",
            "",
            OptimizationLevel::Default,
            RelocMode::Default,
            CodeModel::Default,
        )
        .expect("Failed to create target machine");
    target_machine
        .write_to_file(
            &module,
            FileType::Object,
            Path::new(&format!("{}.o", mod_name)),
        )
        .expect("Failed to write machine code to file");

    // replace with lld
    Command::new("ld")
        .args(&[&format!("{}.o", mod_name), "-o", mod_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("Failed to run ld");

    // println!("Executable generated as 'my_program'");
}
