use ast::{TargetQueryKind, Ty, TyArg};
use internment::Intern;
use parser::query::SourceParseExt;
use typecheck::transform::transform_file;
use typecheck::{BindBody, DefId, FileId, TypedExprKind};

fn transformed(source: &str) -> typecheck::TypedFileAst {
    let output = source.parse_source_full();
    assert!(output.symptoms.is_empty(), "{:?}", output.symptoms);
    transform_file(output.ast, FileId(0))
}

#[test]
fn target_query_lowering_preserves_kind_and_resolved_operand() {
    let typed = transformed("query(element): #alignment(element)\n");
    let query = typed
        .defs
        .get(&DefId(Intern::from_ref("query")))
        .expect("query definition");
    let BindBody::Expr(body) = query.body else {
        panic!("expression body");
    };
    assert!(matches!(
        typed.exprs.kind[body.as_usize()],
        TypedExprKind::TargetQuery {
            kind: TargetQueryKind::Alignment,
            operand: Ty::Opaque(name),
        } if name.as_str() == "element"
    ));
}

#[test]
fn applied_cast_lowering_preserves_nominal_argument_identity() {
    let typed = transformed(
        "Count(unit) is in 0...255\nconvert(value, element): value as Count(element)\n",
    );
    let convert = typed
        .defs
        .get(&DefId(Intern::from_ref("convert")))
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
                && matches!(argument.as_ref(), Ty::Opaque(name) if name.as_str() == "element")
    ));
}
