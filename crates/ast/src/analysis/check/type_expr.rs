use std::collections::{HashMap, HashSet};

use crate::TypeExpr;
use diagnostic::DiagnosticLike;
use diagnostic::type_::TypeSymptom;
use internment::Intern;

use crate::analysis::check::utils::ImportSet;
use crate::analysis::resolve::is_type_surface;
use crate::ty::Ty;

pub(crate) fn check_type_expr(
    tag_types: &HashMap<Intern<String>, Ty>,
    imports: &ImportSet,
    local_names: &HashSet<Intern<String>>,
    type_vars: &HashSet<Intern<String>>,
    e: &TypeExpr,
    symptoms: &mut Vec<diagnostic::Diagnostic>,
) {
    let is_known_locally = |name: &Intern<String>| {
        tag_types.contains_key(name) && (imports.all.contains(name) || local_names.contains(name))
    };
    match e {
        TypeExpr::Nominal(name, span) => {
            if type_vars.contains(name) {
                // Type variable (e.g. `x` in `Range[x].new(start x, end x)`)
            } else if tag_types.get(name).is_none() {
                symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: name.to_string(),
                    }
                    .into_diagnostic(*span),
                );
            } else if !is_known_locally(name) {
                symptoms.push(
                    TypeSymptom::UnknownBinding {
                        name: name.to_string(),
                        did_you_mean: None,
                    }
                    .into_diagnostic(*span),
                );
            }
        }
        TypeExpr::Generic { name, params, span } => {
            if type_vars.contains(name) {
                // Type variable reference (e.g. `Range[x]` where `x` is a var)
            } else if tag_types.get(name).is_none() {
                symptoms.push(
                    TypeSymptom::UnknownTag {
                        name: name.to_string(),
                    }
                    .into_diagnostic(*span),
                );
            } else if !is_known_locally(name) {
                symptoms.push(
                    TypeSymptom::UnknownBinding {
                        name: name.to_string(),
                        did_you_mean: None,
                    }
                    .into_diagnostic(*span),
                );
            }
            for (_, kind) in params {
                if let crate::ParameterKind::Tagged(sp) = kind
                    && let Some(te) = sp.value.as_type_expr()
                    && is_type_surface(&te)
                {
                    check_type_expr(tag_types, imports, local_names, type_vars, &te, symptoms);
                }
            }
        }
        TypeExpr::Qualified(path) if !imports.module_prefixes.contains(&path.root) => {
            symptoms.push(
                TypeSymptom::UnknownBinding {
                    name: path.root.to_string(),
                    did_you_mean: None,
                }
                .into_diagnostic(path.span_id()),
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::check::utils::ImportSet;
    use diagnostic::SpanId;
    use internment::Intern;
    use std::collections::{HashMap, HashSet};

    fn empty_imports() -> ImportSet {
        ImportSet {
            all: HashSet::new(),
            bundle_members: HashSet::new(),
            module_prefixes: HashSet::new(),
            alias_spans: HashSet::new(),
        }
    }

    fn empty_local_names() -> HashSet<Intern<String>> {
        HashSet::new()
    }

    fn empty_type_vars() -> HashSet<Intern<String>> {
        HashSet::new()
    }

    #[test]
    fn known_type_nominal_is_ok() {
        let mut tag_types = HashMap::new();
        let foo = Intern::from_ref("Foo");
        tag_types.insert(foo, crate::ty::Ty::Opaque(foo));
        let mut imports = empty_imports();
        imports.all.insert(foo);
        let local_names = empty_local_names();
        let type_vars = empty_type_vars();
        let expr = TypeExpr::Nominal(foo, SpanId::INVALID);
        let mut symptoms = Vec::new();
        check_type_expr(
            &tag_types,
            &imports,
            &local_names,
            &type_vars,
            &expr,
            &mut symptoms,
        );
        assert!(symptoms.is_empty());
    }

    #[test]
    fn unknown_type_nominal_emits_diagnostic() {
        let mut tag_types = HashMap::new();
        tag_types.insert(
            Intern::from_ref("Foo"),
            crate::ty::Ty::Opaque(Intern::from_ref("Foo")),
        );
        let imports = empty_imports();
        let local_names = empty_local_names();
        let type_vars = empty_type_vars();
        let expr = TypeExpr::Nominal(Intern::from_ref("Bar"), SpanId::INVALID);
        let mut symptoms = Vec::new();
        check_type_expr(
            &tag_types,
            &imports,
            &local_names,
            &type_vars,
            &expr,
            &mut symptoms,
        );
        assert_eq!(symptoms.len(), 1);
        assert!(symptoms[0].message.contains("Bar"));
    }

    #[test]
    fn known_type_not_imported_emits_diagnostic() {
        let mut tag_types = HashMap::new();
        let foo = Intern::from_ref("Foo");
        tag_types.insert(foo, crate::ty::Ty::Opaque(foo));
        // Foo exists in tag_types but is NOT in imports.all
        let imports = empty_imports();
        let local_names = empty_local_names();
        let type_vars = empty_type_vars();
        let expr = TypeExpr::Nominal(foo, SpanId::INVALID);
        let mut symptoms = Vec::new();
        check_type_expr(
            &tag_types,
            &imports,
            &local_names,
            &type_vars,
            &expr,
            &mut symptoms,
        );
        assert_eq!(symptoms.len(), 1);
        assert!(symptoms[0].message.contains("Foo"));
    }

    #[test]
    fn known_type_local_definition_is_ok() {
        let mut tag_types = HashMap::new();
        let foo = Intern::from_ref("Foo");
        tag_types.insert(foo, crate::ty::Ty::Opaque(foo));
        // Foo is defined locally, not imported
        let imports = empty_imports();
        let mut local_names = HashSet::new();
        local_names.insert(foo);
        let type_vars = empty_type_vars();
        let expr = TypeExpr::Nominal(foo, SpanId::INVALID);
        let mut symptoms = Vec::new();
        check_type_expr(
            &tag_types,
            &imports,
            &local_names,
            &type_vars,
            &expr,
            &mut symptoms,
        );
        assert!(symptoms.is_empty());
    }
}
