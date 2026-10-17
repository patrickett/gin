use ast::{BindValue, Expr, WhenArm};
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn imports_remain_in_their_branch_instead_of_becoming_file_imports() {
    let output =
        "main: when 1 = 1 then\n    use dev.tools as tools\n    tools.show_wireframe\nelse 7\n"
            .parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    assert!(output.ast.uses.is_empty());
    let BindValue::Expr(expr) = &output.ast.defs[&Intern::from_ref("main")].value else {
        panic!()
    };
    let Expr::When(when) = &expr.value else {
        panic!()
    };
    let WhenArm::Cond { body, .. } = &when.arms[0] else {
        panic!()
    };
    let Expr::Block(expressions) = &body.value else {
        panic!()
    };
    assert_eq!(expressions.len(), 2);
    let Expr::Import(import) = &expressions[0].value else {
        panic!()
    };
    assert_eq!(import.0[0].alias.unwrap().as_str(), "tools");
}

#[test]
fn inactive_imports_still_require_valid_syntax() {
    let output = "main: when 0 = 1 then\n    use\nelse 7\n".parse_source_full();
    assert!(!output.symptoms.is_empty());
}

#[test]
fn conditional_import_fixture_parses_without_special_when_syntax() {
    let source = include_str!("../../test-fixtures/fixtures/conditional_imports/main.gin");
    let output = source.parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    assert_eq!(output.ast.uses.len(), 2);
    assert!(output.ast.defs.contains_key(&Intern::from_ref("run")));
}
