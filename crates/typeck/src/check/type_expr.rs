use ast::{Expr, HasSpanId};
use diagnostic::type_::TypeSymptom;
use diagnostic::DiagnosticLike;

use crate::resolve::is_type_surface;
use crate::TyEnv;

impl TyEnv {
    pub(crate) fn check_type_expr(
        &self,
        e: &Expr,
        symptoms: &mut Vec<diagnostic::Diagnostic>,
    ) {
        match e {
            Expr::TypeNominal(name, span) if self.lookup_tag(*name).is_none() => {
                symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: name.to_string(),
                    }
                    .into_diagnostic(*span),
                );
            }
            Expr::TypeGeneric { name, params, span } => {
                if self.lookup_tag(*name).is_none() {
                    symptoms.push(
                        TypeSymptom::UnknownTag {
                            name: name.to_string(),
                        }
                        .into_diagnostic(*span),
                    );
                }
                for (_, kind) in params {
                    if let ast::ParameterKind::Tagged(sp) = kind
                        && is_type_surface(&sp.0)
                    {
                        self.check_type_expr(&sp.0, symptoms);
                    }
                }
            }
            Expr::TypeQualified(path) if self.lookup_tag(path.root).is_none() => {
                symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: path.root.to_string(),
                    }
                    .into_diagnostic(path.span_id()),
                );
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use diagnostic::SpanId;
    use internment::Intern;

    #[test]
    fn known_type_nominal_is_ok() {
        let src = "Foo is Foo\n";
        let ast = parser::parse_source_full(src);
        let env = TyEnv::from_file_ast(&ast.ast);
        let expr = Expr::TypeNominal(Intern::from_ref("Foo"), SpanId::INVALID);
        let mut symptoms = Vec::new();
        env.check_type_expr(&expr, &mut symptoms);
        assert!(symptoms.is_empty());
    }

    #[test]
    fn unknown_type_nominal_emits_diagnostic() {
        let src = "Foo is Foo\n";
        let ast = parser::parse_source_full(src);
        let env = TyEnv::from_file_ast(&ast.ast);
        let expr = Expr::TypeNominal(Intern::from_ref("Bar"), SpanId::INVALID);
        let mut symptoms = Vec::new();
        env.check_type_expr(&expr, &mut symptoms);
        assert_eq!(symptoms.len(), 1);
        assert!(symptoms[0].message.contains("Bar"));
    }
}
