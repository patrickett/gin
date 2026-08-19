use internment::Intern;
use parser::cursor::TokenCursor;
use typecheck::transform::{TransformCtx, transform};
use typecheck::{BindBody, DefId, FileId, TypedFileAst};

fn check(source: &str) -> TypedFileAst {
    transform(
        &TokenCursor::parse_source(source),
        FileId(0),
        &TransformCtx::new(),
    )
}

fn body(typed: &TypedFileAst, name: &str) -> typecheck::ExprId {
    match &typed.defs[&DefId(Intern::new(name.to_string()))].body {
        BindBody::Expr(expr) => *expr,
        BindBody::Body { exprs, ret } => ret.or_else(|| exprs.last().copied()).unwrap(),
        BindBody::Extern => panic!("expected a body"),
    }
}

#[test]
fn renamed_fixed_array_former_retains_named_application_and_array_repr() {
    let typed = check(
        "Byte is in 0...255\n\
         #fixed_array(element, count)\n\
         LibraryBuffer(element Type, count BigInt) has\n\
         make(value Byte) LibraryBuffer(Byte, 4): (value; 4)\n",
    );
    let result = &typed.exprs.ty[body(&typed, "LibraryBuffer.make").as_usize()];
    eprintln!("DEBUG result type: {:#?}", result);
    let former = &typed.tags[&typecheck::TagId(Intern::from_ref("LibraryBuffer"))].resolved_ty;
    assert_eq!(result.type_id(), former.type_id());
    assert_eq!(
        typecheck::representation::Repr::derive(result, Some(&typed.type_registry)).ok(),
        Some(typecheck::representation::Repr::Array {
            element: Box::new(typecheck::representation::Repr::Bits { width: 8 }),
            length: 4,
        })
    );
}

#[test]
fn uncontextualized_fixed_array_construction_infers_structural_array() {
    let typed = check(
        "Byte is in 0...255\n\
         #fixed_array(element, count)\n\
         LibraryBuffer(element Type, count BigInt) has\n\
         make(value Byte): (value; 4)\n",
    );
    let result = &typed.exprs.ty[body(&typed, "LibraryBuffer.make").as_usize()];
    assert!(matches!(result, typecheck::ty::Ty::Array { .. }));
}
