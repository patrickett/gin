mod support;

use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::{
    DefId, FileId, TypedFileAst,
    transform::{TransformCtx, transform},
};

fn codes(typed: &TypedFileAst) -> Vec<String> {
    typed
        .all_flaws()
        .into_iter()
        .map(|(_, d)| d.code.to_string())
        .collect()
}

#[test]
fn trait_operator_implementation_inherits_attributes_and_return_type() {
    let typed = support::check(
        r#"
Add has
    #infix('+'), #inline
    add(self, rhs Self) Self
Word is in 0 to 255
Word has Add
    add(self, rhs Self):
        left is in 0 to 255: self
        right is in 0 to 255: rhs
        return ((left + right) & 255) as Self
left Word: 1
right Word: 2
result: left + right
"#,
    );
    assert!(codes(&typed).is_empty(), "{:?}", codes(&typed));
    let method = &typed.defs[&DefId::named(Intern::from_ref("Word.add"))];
    assert_eq!(
        method.infix_symbol.as_deref().map(String::as_str),
        Some("+")
    );
    assert!(method.attributes.inline_always);
    assert_eq!(method.return_type, method.params[0].1);
}

#[test]
fn arithmetic_operators_accept_checked_results_preserving_the_operand_type() {
    for operator in ["+", "-", "*"] {
        let source = format!(
            "Arithmetic(result) has\n    #infix('{operator}')\n    calculate(self, rhs Self) result\n\
             Word is in 0 to 255\nOutcome is Word or Flaw(Overflow or Underflow)\n\
             Word has Arithmetic(Outcome)\n\
                 \x20   calculate(self is in Self, rhs is in Self):\n\
                 \x20       candidate: self {operator} rhs\n\
                 \x20       return when candidate is in Word then candidate as Self else Flaw(Overflow)\n"
        );
        for qualified in [false, true] {
            let mut ast = TokenCursor::parse_source(&source);
            if qualified {
                ast = ast.qualify_module_defs("numbers.checked");
            }
            let typed = transform(&ast, FileId(0), &TransformCtx::new());
            support::expect_no_flaws(&typed);
        }
    }
}

#[test]
fn checked_arithmetic_results_reject_unrelated_success_types() {
    for outcome in ["Other or Flaw", "Word or Other or Flaw", "Flaw or Missing"] {
        let typed = support::check(&format!(
            "Arithmetic(result) has\n    #infix('+')\n    calculate(self, rhs Self) result\n\
             Word is in 0 to 255\nOther is in 0 to 255\nOutcome is {outcome}\n\
             Word has Arithmetic(Outcome)\n\
                 \x20   calculate(self is in Self, rhs is in Self): return Flaw\n"
        ));
        assert!(
            codes(&typed).contains(&"type-invalid-operator-result".into()),
            "{outcome}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn bitwise_operators_preserve_the_operand_type_without_checked_results() {
    for operator in ["&", "|", "^"] {
        let typed = support::check(&format!(
            "Bits(result) has\n    #infix('{operator}')\n    calculate(self, rhs Self) result\n\
             Word is in 0 to 255\nOutcome is Word or Flaw\nWord has Bits(Outcome)\n\
                 \x20   calculate(self is in Self, rhs is in Self): return (self {operator} rhs) as Self\n"
        ));
        assert!(
            codes(&typed).contains(&"type-invalid-operator-result".into()),
            "{operator}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn missing_and_misspelled_methods_are_rejected() {
    for body in ["", "    other(self, rhs Self) Self: return rhs\n"] {
        let typed = support::check(&format!(
            "Eq has equals(self, rhs Self) Self\nWord is in 0 to 255\nWord has Eq\n    finish(self):\n        return\n{body}"
        ));
        assert!(
            codes(&typed).contains(&"type-missing-trait-method".into()),
            "{:?}",
            codes(&typed)
        );
    }
}

#[test]
fn incompatible_method_contracts_are_rejected() {
    for implementation in [
        "add(self) Self: return self",
        "add(self, rhs Other) Self: return self",
        "add(self ref, rhs Self) Self: return rhs",
        "add(self, rhs Self) Other: return 1 as Other",
    ] {
        let typed = support::check(&format!(
            "Add has add(self, rhs Self) Self\nOther is in 0 to 10\nWord is in 0 to 255\nWord has Add\n    finish(self):\n        return\n    {implementation}\n"
        ));
        assert!(
            codes(&typed).contains(&"type-trait-method-mismatch".into()),
            "{implementation}: {:?}",
            codes(&typed)
        );
    }
}

#[test]
fn qualified_implementations_resolve_conflicting_contracts() {
    let typed = support::check(
        "A has run(self ref) First\nB has run(self ref) Second\nFirst is in 0 to 10\nFirst has\n    finish(self):\n        return\nSecond is in 0 to 20\nSecond has\n    finish(self):\n        return\nWord is in 0 to 255\nWord has A and B\n    finish(self):\n        return\n    A.run(self ref): return 1 as First\n    B.run(self ref): return 2 as Second\n",
    );
    assert!(codes(&typed).is_empty(), "{:?}", codes(&typed));
}

#[test]
fn unqualified_conflicting_contracts_require_qualification() {
    let typed = support::check(
        "A has run(self ref) First\nB has run(self ref) Second\nFirst is in 0 to 10\nFirst has\n    finish(self):\n        return\nSecond is in 0 to 20\nSecond has\n    finish(self):\n        return\nWord is in 0 to 255\nWord has A and B\n    finish(self):\n        return\n    run(self ref): return 1 as First\n",
    );
    assert!(
        codes(&typed).contains(&"type-trait-method-mismatch".into()),
        "{:?}",
        codes(&typed)
    );
}

#[test]
fn inherited_return_type_checks_the_implementation_body() {
    let typed = support::check(
        "A has run(self ref) First\nFirst is in 0 to 10\nFirst has\n    finish(self):\n        return\nSecond is in 0 to 20\nSecond has\n    finish(self):\n        return\nWord is in 0 to 255\nWord has A\n    finish(self):\n        return\n    run(self ref): return 2 as Second\n",
    );
    assert!(
        codes(&typed).contains(&"type-return-mismatch".into()),
        "{:?}",
        codes(&typed)
    );
}

#[test]
fn composed_traits_inherit_operator_contracts() {
    let typed = support::check(
        r#"
Add has
    #infix('+')
    add(self, rhs Self) Self
Subtract has
    #infix('-')
    subtract(self, rhs Self) Self
Arithmetic is Add and Subtract
Word is in 0 to 255
Word has Arithmetic
    add(self, rhs Self):
        left is in 0 to 255: self
        right is in 0 to 255: rhs
        return ((left + right) & 255) as Self
    subtract(self, rhs Self):
        left is in 0 to 255: self
        right is in 0 to 255: rhs
        return ((left - right) & 255) as Self
"#,
    );
    assert!(codes(&typed).is_empty(), "{:?}", codes(&typed));
    assert_eq!(
        typed.defs[&DefId::named(Intern::from_ref("Word.subtract"))].infix_symbol,
        Some(Intern::from_ref("-"))
    );
}

#[test]
fn conflicting_operator_attributes_are_rejected() {
    let typed = support::check(
        r#"
Add has
    #infix('+')
    add(self, rhs Self) Self
Word is in 0 to 255
Word has Add
    #infix('-')
    add(self, rhs Self) Self: return rhs
"#,
    );
    assert!(
        codes(&typed).contains(&"type-trait-method-mismatch".into()),
        "{:?}",
        codes(&typed)
    );
}

#[test]
fn imported_trait_contracts_are_available_to_implementations() {
    let contract = TokenCursor::parse_source(
        "Add has\n    #infix('+'), #inline\n    add(self, rhs Self) Self\n",
    );
    let declared = transform(&contract, FileId(0), &TransformCtx::new());
    let mut ctx = TransformCtx::from_typed_asts(&[&declared]);
    ctx.compile_time_eval_ast = std::sync::Arc::new(contract);
    let implementation = TokenCursor::parse_source(
        "use contracts Add\nWord is in 0 to 255\nWord has Add\n    add(self, rhs Self):\n        left is in 0 to 255: self\n        right is in 0 to 255: rhs\n        return ((left + right) & 255) as Self\n",
    );
    let typed = transform(&implementation, FileId(1), &ctx);
    assert!(codes(&typed).is_empty(), "{:?}", codes(&typed));
    let method = &typed.defs[&DefId::named(Intern::from_ref("Word.add"))];
    assert_eq!(method.infix_symbol, Some(Intern::from_ref("+")));
    assert!(method.attributes.inline_always);
}

#[test]
fn named_equality_trait_dispatches_to_the_explicit_implementation() {
    let typed = support::check(
        r#"
Eq has
    #inline
    equals(self, rhs Self) True or False
Word is in 0 to 255
Word has Eq
    equals(self, rhs Self):
        left is in 0 to 255: self
        right is in 0 to 255: rhs
        return when left = right then True else False
compare(left Word, right Word) True or False: return left.equals(right)
"#,
    );
    assert!(codes(&typed).is_empty(), "{:?}", codes(&typed));
    let method = &typed.defs[&DefId::named(Intern::from_ref("Word.equals"))];
    assert_eq!(method.infix_symbol, None);
    let typecheck::BindBody::Expr(body) =
        typed.defs[&DefId::named(Intern::from_ref("compare"))].body
    else {
        panic!("expected comparison body");
    };
    assert!(
        matches!(&typed.exprs.kind[body.as_usize()], typecheck::TypedExprKind::FnCall { target, .. } if target.0.as_str() == "Word.equals")
    );
}

#[test]
fn qualified_methods_must_name_an_inherited_trait_method() {
    for (name, expected) in [
        ("A.typo", "type-unknown-trait-method"),
        ("B.run", "type-uninherited-trait-qualifier"),
    ] {
        let typed = support::check(&format!(
            "A has run(self ref) Word\nB has run(self ref) Word\nWord is in 0 to 255\nWord has A\n    finish(self):\n        return\n    {name}(self ref) Word: return 1 as Word\n"
        ));
        assert!(
            codes(&typed).contains(&expected.into()),
            "{:?}",
            codes(&typed)
        );
    }
}

#[test]
fn compatible_traits_can_share_one_explicit_implementation() {
    let typed = support::check(
        "A has run(self ref) Word\nB has run(self ref) Word\nWord is in 0 to 255\nWord has A and B\n    finish(self):\n        return\n    run(self ref): return 1 as Word\n",
    );
    assert!(codes(&typed).is_empty(), "{:?}", codes(&typed));
}

#[test]
fn module_qualification_preserves_trait_method_identity() {
    for method in ["add", "Add.add"] {
        let source = format!(
            r#"
Add has
    #infix('+'), #inline
    add(self, rhs Self) Self
Word is in 0 to 255
Word has Add
    finish(self):
        return
    {method}(self, rhs Self):
        left is in 0 to 255: self
        right is in 0 to 255: rhs
        return ((left + right) & 255) as Self
"#
        );
        let ast = TokenCursor::parse_source(&source).qualify_module_defs("numbers.integer");
        let typed = transform(&ast, FileId(0), &TransformCtx::new());
        assert!(codes(&typed).is_empty(), "{:?}", codes(&typed));
        let name = Intern::new(format!("numbers.integer.Word.{method}"));
        let bind = &typed.defs[&DefId::named(name)];
        assert_eq!(bind.infix_symbol, Some(Intern::from_ref("+")));
        assert!(bind.attributes.inline_always);
    }
}
