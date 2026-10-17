use ast::{TargetQueryKind, Ty, TyArg};
use parser::query::SourceParseExt;
use typecheck::transform::transform_file;
use typecheck::{BindBody, FileId, TypedExprKind};

fn transformed(source: &str) -> typecheck::TypedFileAst {
    let output = source.parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    transform_file(output.ast, FileId(0))
}

#[test]
fn target_query_lowering_preserves_kind_and_resolved_operand() {
    let typed = transformed(
        "Byte is in 0 to 255\nByte has finish(self): return\nquery(element): return #alignment(element)\ncheck: query(Byte)\n",
    );
    let query = typed
        .defs
        .values()
        .find(|bind| bind.name.as_str().starts_with("query$specialized$"))
        .expect("query definition");
    let BindBody::Expr(body) = query.body else {
        panic!("expression body");
    };
    assert!(matches!(
        typed.exprs.kind[body.as_usize()],
        TypedExprKind::TargetQuery {
            kind: TargetQueryKind::Alignment,
            operand: Ty::Named { name, .. },
        } if name.as_str() == "Byte"
    ));
}

#[test]
fn applied_cast_lowering_preserves_nominal_argument_identity() {
    let typed = transformed(
        "Byte is in 0 to 255\nByte has finish(self): return\nCount(unit) is in 0 to 255\nCount(unit) has finish(self): return\nconvert(value is in 0 to 255, element): return value as Count(element)\ncheck: convert(7, Byte)\n",
    );
    let convert = typed
        .defs
        .values()
        .find(|bind| bind.name.as_str().starts_with("convert$specialized$"))
        .expect("convert definition");
    let BindBody::Expr(body) = convert.body else {
        panic!("expression body");
    };
    let TypedExprKind::Cast { ty, .. } = &typed.exprs.kind[body.as_usize()] else {
        panic!("cast expression");
    };
    let Ty::Named { instance, .. } = ty else {
        panic!("named Count application: {ty:?}");
    };
    assert!(matches!(
        instance.arguments.as_slice(),
        [(parameter, TyArg::Type(argument))]
            if parameter.as_str() == "unit"
                && matches!(argument.as_ref(), Ty::Named { name, .. } if name.as_str() == "Byte")
    ));
}
