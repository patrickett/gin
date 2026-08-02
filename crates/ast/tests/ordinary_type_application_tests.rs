//! Ordinary expressions in generic type applications.

mod support;

use ast::NormalExpr;
use support::transform_source;
use typecheck::ty::Ty;
use typecheck::typed::TagId;

#[test]
fn binary_type_argument_resolves_as_array_size() {
    let typed = transform_source(
        "Array(x Type, n Int) has size Int\nHolder has values Array(Int, n + 1)\n",
    );
    assert!(
        typed.all_flaws().is_empty(),
        "ordinary type argument should resolve without flaws: {:?}",
        typed.all_flaws()
    );

    let holder = typed
        .tags
        .get(&TagId(internment::Intern::from_ref("Holder")))
        .expect("Holder");
    let Ty::Record { fields, .. } = &holder.resolved_ty else {
        panic!(
            "Holder should resolve to a record: {:?}",
            holder.resolved_ty
        );
    };
    let values = fields
        .iter()
        .find(|(name, _)| name.as_str() == "values")
        .expect("values field");
    assert!(
        matches!(
            values.1.as_ref(),
            Ty::Array {
                size: NormalExpr::Var(_),
                ..
            }
        ),
        "resolved values field: {:?}",
        values.1
    );
}
