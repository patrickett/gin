use ast::ConstValue;
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::staging::{NormalizeResult, normalize};
use typecheck::subst::DepSubst;
use typecheck::{BindBody, CompileTimeHost, DefId, EvalOutcome, Evaluator, FileId};

#[test]
fn comparison_values_preserve_widths_and_signedness_in_both_evaluation_paths() {
    for (expression, expected) in [
        ("1 < 2", true),
        ("1 = 3", false),
        ("2 > 1", true),
        ("255 < 256", true),
        ("256 <= 255", false),
        ("-1 < 255", true),
        ("255 > -1", true),
        ("-129 >= 127", false),
        ("-128 < -1", true),
        ("-1 = 255", false),
        ("340282366920938463463374607431768211455 > -1", true),
        ("-1 < 340282366920938463463374607431768211455", true),
    ] {
        let source = format!("comparison: {expression}\n");
        let parsed = source.parse_source_full();
        assert!(
            parsed.symptoms.is_empty(),
            "{expression}: {:?}",
            parsed.symptoms
        );
        let mut typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
        assert!(
            typed.all_flaws().is_empty(),
            "{expression}: {:?}",
            typed.all_flaws()
        );
        let definition = DefId::named(Intern::from_ref("comparison"));
        let BindBody::Expr(root) = typed.defs[&definition].body else {
            panic!("comparison expression");
        };
        typed.exprs.facts.const_facts.const_value.fill(None);
        let target = flask::CompileTarget::Library;
        let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
        let outcome = evaluator.eval(root).unwrap();
        assert!(
            matches!(&outcome, EvalOutcome::Complete(ConstValue::Proposition(actual))
            if *actual == expected),
            "{expression}: {outcome:?}"
        );
        assert_eq!(
            normalize(&typed, root, &DepSubst::new()),
            NormalizeResult::Value(ConstValue::Proposition(expected)),
            "{expression}"
        );
    }
}

#[test]
fn comparison_arguments_preserve_each_nominal_integer_domain() {
    let source = concat!(
        "Unsigned is in 0 to 255\nUnsigned has finish(self): return\n",
        "Signed is in -128 to 127\nSigned has finish(self): return\n",
        "less(left Signed, right Unsigned): left < right\n",
        "greater(left Unsigned, right Signed): left > right\n",
    );
    let parsed = source.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let typed = typecheck::transform::transform_file(parsed.ast, FileId(0));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let target = flask::CompileTarget::Library;
    for (name, arguments) in [("less", [-1, 255]), ("greater", [255, -1])] {
        let mut evaluator = Evaluator::new(typed.resolved_program(), CompileTimeHost::new(&target));
        let definition = DefId::named(Intern::from_ref(name));
        let arguments = arguments.map(|value| ConstValue::Int(value.into()));
        assert_eq!(
            evaluator.call(definition, &arguments),
            Ok(ConstValue::Proposition(true)),
            "{name}"
        );
    }
}
