use ast::Ty;
use internment::Intern;
use parser::parse_from_str;
use typecheck::representation::Repr;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId, TypedFileAst};

const TYPES: &str = "Byte is in 0 to 255\nLength is in 0 to 18446744073709551615\nSlice(element) has ptr @element, len Length\n";

fn check(source: &str) -> TypedFileAst {
    let parsed = parse_from_str(source);
    assert!(
        parsed.parse_warnings.is_empty(),
        "{:?}",
        parsed.parse_warnings
    );
    transform(&parsed, FileId(0), &TransformCtx::new())
}

#[test]
fn text_literals_are_utf8_byte_arrays_without_declarations() {
    for (literal, length) in [("'é🦀'", 6), ("\"é🦀\"", 6), ("''", 0), ("'a\\0b'", 3)] {
        let typed = check(&format!("bytes: {literal}\n"));
        assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
        let bind = &typed.defs[&DefId(Intern::from_ref("bytes"))];
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
    let typed = check(&format!(
        "{TYPES}\nmake:\n    bytes := 'hello'\n    view Slice(Byte): Slice(ptr: (@bytes) as @Byte, len: #size(bytes) as Length)\nreturn view.len\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
}

#[test]
fn a_text_literal_cannot_implicitly_construct_a_record() {
    let typed = check(&format!("{TYPES}\ntext Slice(Byte): 'hello'\n"));
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
    let typed = check("first: 'é🦀'.(0)\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let BindBody::Expr(expr) = typed.defs[&DefId(Intern::from_ref("first"))].body else {
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
    let typed = check("Byte is in 0 to 255\naddress @Byte: (@'hello') as @Byte\n");
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(matches!(
        typed.defs[&DefId(Intern::from_ref("address"))].return_type,
        Ty::Ptr { .. }
    ));
}
