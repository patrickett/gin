mod support;

use ast::Ty;
use internment::Intern;
use support::check_lexed;
use typecheck::representation::Repr;
use typecheck::{BindBody, DefId};

const TYPES: &str = "Byte is in 0 to 255\nByte has\n    finish(self):\n        return\nLength is in 0 to 18446744073709551615\nLength has\n    finish(self):\n        return\nSlice(element) has\n    ptr @element, len Length\n    finish(self):\n        return\n";

#[test]
fn text_literals_are_utf8_byte_arrays_without_declarations() {
    for (literal, length) in [("'é🦀'", 6), ("\"é🦀\"", 6), ("''", 0), ("'a\\0b'", 3)] {
        let typed = check_lexed(&format!("bytes: {literal}\n"));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let bind = &typed.defs[&DefId::named(Intern::from_ref("bytes"))];
        assert_eq!(
            Repr::derive(&bind.return_type, Some(&typed.type_registry)),
            Ok(Repr::Array {
                element: Box::new(Repr::Bits { width: 8 }),
                length
            })
        );
    }
}

#[test]
fn ordinary_slice_construction_uses_array_address_and_layout() {
    let typed = check_lexed(&format!(
        "{TYPES}\nmake:\n    bytes := 'hello'\n    view Slice(Byte): Slice(ptr: (@bytes) as @Byte, len: #size(bytes) as Length)\n    return view.len\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn a_text_literal_cannot_implicitly_construct_a_record() {
    let typed = check_lexed(&format!("{TYPES}\ntext Slice(Byte): 'hello'\n"));
    assert!(
        typed
            .all_flaws()
            .iter()
            .any(|(_, flaw)| flaw.code.slug() == "type-text-literal-mismatch"),
        "{:?}",
        typed.all_flaws()
    );
}

#[test]
fn literal_indexing_evaluates_utf8_bytes() {
    let typed = check_lexed("first: 'é🦀'.[0]\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let BindBody::Expr(expr) = typed.defs[&DefId::named(Intern::from_ref("first"))].body else {
        panic!("expression")
    };
    let target = flask::CompileTarget::Library;
    let result = typecheck::Evaluator::new(
        typed.resolved_program(),
        typecheck::CompileTimeHost::new(&target),
    )
    .eval(expr)
    .unwrap();
    assert!(
        matches!(result, typecheck::EvalOutcome::Complete(ast::ConstValue::Int(value)) if value == 195.into()),
        "{result:?}"
    );
}

#[test]
fn taking_a_literal_address_requires_no_slice_type() {
    let typed = check_lexed(&format!(
        "{}address @Byte: (@'hello') as @Byte\n",
        test_fixtures::preludes::BYTE_PRELUDE
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(matches!(
        typed.defs[&DefId::named(Intern::from_ref("address"))].return_type,
        Ty::Ptr { .. }
    ));
}
