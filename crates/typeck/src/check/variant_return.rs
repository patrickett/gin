use ast::{Bind, BindValue, Expr, HasSpanId, Spanned, WhenArm};
use diagnostic::type_::TypeSymptom;
use diagnostic::DiagnosticLike;
use internment::Intern;

pub(crate) fn check_return_variants(
    bind: &Bind,
    valid_variants: &[Intern<String>],
    union_name: Intern<String>,
    symptoms: &mut Vec<diagnostic::Diagnostic>,
) {
    fn check_expr(
        expr: &Spanned<Expr>,
        valid_variants: &[Intern<String>],
        union_name: Intern<String>,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
    ) {
        match &expr.0 {
            Expr::AnonymousTag(name, span)
                if !valid_variants.iter().any(|v| v.as_str() == name.as_str()) =>
            {
                symptoms.push(
                    TypeSymptom::NotAVariant {
                        name: name.to_string(),
                        union_name: union_name.to_string(),
                    }
                    .into_diagnostic(*span),
                );
            }
            Expr::TagCall(tc)
                if !valid_variants
                    .iter()
                    .any(|v| v.as_str() == tc.name.as_str()) =>
            {
                symptoms.push(
                    TypeSymptom::NotAVariant {
                        name: tc.name.to_string(),
                        union_name: union_name.to_string(),
                    }
                    .into_diagnostic(tc.span_id()),
                );
            }
            Expr::If(if_expr) => {
                for e in &if_expr.body {
                    check_expr(e, valid_variants, union_name, symptoms);
                }
                if let Some(ret_expr) = &if_expr.ret.0 {
                    check_expr(ret_expr, valid_variants, union_name, symptoms);
                } else {
                    symptoms.push(
                        TypeSymptom::EmptyReturn {
                            expected_type: union_name.to_string(),
                        }
                        .into_diagnostic(expr.1),
                    );
                }
            }
            Expr::When(w) => {
                for arm in &w.arms {
                    match arm {
                        WhenArm::Cond { body, .. } => {
                            check_expr(body, valid_variants, union_name, symptoms)
                        }
                        WhenArm::Is { body, .. } => {
                            check_expr(body, valid_variants, union_name, symptoms)
                        }
                        WhenArm::Else(body) => {
                            check_expr(body, valid_variants, union_name, symptoms)
                        }
                    }
                }
            }
            Expr::Bind(inner) => match inner.value() {
                BindValue::Expr(e) => check_expr(e, valid_variants, union_name, symptoms),
                BindValue::Body { exprs, ret } => {
                    for e in exprs {
                        check_expr(e, valid_variants, union_name, symptoms);
                    }
                    if let Some(r) = &ret.0 {
                        check_expr(r, valid_variants, union_name, symptoms);
                    } else {
                        symptoms.push(
                            TypeSymptom::EmptyReturn {
                                expected_type: union_name.to_string(),
                            }
                            .into_diagnostic(inner.name_span),
                        );
                    }
                }
                BindValue::Extern => {}
            },
            _ => {}
        }
    }

    match bind.value() {
        BindValue::Expr(expr) => check_expr(expr, valid_variants, union_name, symptoms),
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                check_expr(expr, valid_variants, union_name, symptoms);
            }
            if let Some(ret_expr) = &ret.0 {
                check_expr(ret_expr, valid_variants, union_name, symptoms);
            } else {
                symptoms.push(
                    TypeSymptom::EmptyReturn {
                        expected_type: union_name.to_string(),
                    }
                    .into_diagnostic(bind.name_span),
                );
            }
        }
        BindValue::Extern => {}
    }
}
