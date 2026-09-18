use ast::ty::PackageInstanceKey;
use flask::{CompileTarget, TargetTriple};
use ginc::driver::{CompilePackage, CompilerDriver, SourceFile, SourcePackage};
use std::collections::HashMap;
use test_fixtures::TempPackage;

#[test]
fn imported_operator_calls_a_library_owned_instruction_body() {
    let dependency = TempPackage::new("instruction_operator_library");
    dependency.write_flask("arithmetic");
    dependency.write("machine/instructions.gin", MACHINE);
    dependency.write(
        "numeric/number.gin",
        r#"use '../machine' Word, machine_sum
Number is in -9223372036854775808 to 9223372036854775807
#infix('+')
sum(a Number, b Number) Number: machine_sum(a as Word, b as Word) as Number
"#,
    );
    let consumer = TempPackage::new("instruction_operator_consumer");
    consumer.write_flask_with_deps(
        "client",
        &format!(
            r#"{{"arithmetic":{{"path":"../{}"}}}}"#,
            dependency.path().file_name().unwrap().to_string_lossy()
        ),
    );
    let source = "use arithmetic.numeric Number, sum\ncalculate(a Number, b Number) Number: a + b\nconstant_result() Number: (20 as Number) + (22 as Number)\nwrapped_result() Number: (9223372036854775807 as Number) + (1 as Number)\n";
    let main = consumer.write("main.gin", source);
    let mut driver = CompilerDriver::default();
    let parsed = driver
        .parse(SourcePackage::new(
            main.clone(),
            vec![SourceFile::new(main.clone(), source)],
        ))
        .unwrap();
    let mut dependencies = HashMap::new();
    dependencies.insert(
        "arithmetic".to_string(),
        dependency.path().canonicalize().unwrap(),
    );
    let package = driver.check(
        CompilePackage::new(
            parsed,
            CompileTarget::Concrete(TargetTriple::parse("x86_64-unknown-linux-gnu").unwrap()),
        )
        .with_dependencies(dependencies)
        .with_import_resolution(true)
        .with_package_fallback(PackageInstanceKey::workspace("instruction-client", "1")),
    );
    let diagnostics = package
        .files()
        .iter()
        .flat_map(|file| {
            file.front_end_diagnostics()
                .iter()
                .chain(file.typecheck_diagnostics())
        })
        .map(|diagnostic| diagnostic.message.clone())
        .collect::<Vec<_>>();
    assert!(
        !package.has_front_end_flaws() && !package.has_typecheck_flaws(),
        "{diagnostics:?}"
    );
    let assembly = package
        .files()
        .iter()
        .flat_map(|file| file.typed().exprs.kind.iter())
        .find_map(|kind| {
            if let typecheck::TypedExprKind::Asm {
                state: typecheck::assembly::AssemblyState::Checked(assembly),
                ..
            } = kind
            {
                Some(assembly)
            } else {
                None
            }
        })
        .expect("checked instruction body from imported library");
    assert_eq!(assembly.template, ".byte 72, 1, 200");
    for (name, expected) in [("constant_result", 42_i64), ("wrapped_result", i64::MIN)] {
        let (file, bind) = package
            .files()
            .iter()
            .find_map(|file| {
                file.typed()
                    .defs
                    .values()
                    .find(|bind| bind.name.as_str().rsplit('.').next() == Some(name))
                    .map(|bind| (file.typed(), bind))
            })
            .expect("consumer function");
        let typecheck::BindBody::Expr(expr) = bind.body else {
            panic!("consumer expression");
        };
        assert_eq!(
            file.exprs.facts.const_facts.const_value[expr.as_usize()],
            Some(ast::ConstValue::Int(expected.into())),
            "{name}: {:?}",
            file.exprs.kind_of(expr)
        );
    }
}

const MACHINE: &str = r#"Word is in -9223372036854775808 to 9223372036854775807
Count is in 0 to 255
Choice(slot_type Type, register_type Type) has slot slot_type, register register_type, code Count
Access has pointer Count, length Count
Instruction(encoder_type Type, arguments_type Type) has encoder encoder_type, size Count, arguments arguments_type
Region(contract_type Type, items_type Type) has contract contract_type, items items_type
Encoder has
    encode() Sequence(Count, 3): [72 as Count, 1 as Count, 200 as Count]
Contract(target_type Type, choices_type Type, inputs_type Type, outputs_type Type, clobbers_type Type, reads_type Type, writes_type Type) has
    target target_type
    choices choices_type
    inputs inputs_type
    outputs outputs_type
    clobbers clobbers_type
    reads reads_type
    writes writes_type
    evaluate(a is in -9223372036854775808 to 9223372036854775807, b is in -9223372036854775808 to 9223372036854775807) Word: (a + b) as Word

machine_sum(a Word, b Word) left Word: #asm(region(), left: a, right: b)
contract():
return Contract(
        target: #target.triple
        choices: (choice0: Choice(slot: 'left', register: 'rax', code: 0), choice1: Choice(slot: 'right', register: 'rcx', code: 1))
        inputs: (input0: 'left', input1: 'right')
        outputs: ['left']
        clobbers: ['cc']
        reads: ()
        writes: ()
    )
region(): Region(contract: contract(), items: (instruction0: Instruction(encoder: Encoder, size: 3, arguments: ())))

#fixed_array(element, count)
Sequence(element Type, count BigInt) has
"#;
