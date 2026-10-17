use ast::{Declare, TagMap};
use internment::Intern;

#[derive(Clone, Copy)]
pub(crate) struct TagDeclarations<'a> {
    pub(crate) local: &'a TagMap,
    pub(crate) package: Option<&'a TagMap>,
}

impl<'a> TagDeclarations<'a> {
    pub(crate) fn overlay(local: &'a TagMap, package: &'a TagMap) -> Self {
        Self {
            local,
            package: Some(package),
        }
    }

    pub(crate) fn get(&self, name: &Intern<String>) -> Option<&'a Declare> {
        self.local
            .get(name)
            .or_else(|| self.package.and_then(|package| package.get(name)))
    }
}

impl<'a> From<&'a TagMap> for TagDeclarations<'a> {
    fn from(local: &'a TagMap) -> Self {
        Self {
            local,
            package: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::TypeEnv;
    use parser::query::SourceParseExt;
    use std::collections::HashMap;

    #[test]
    fn borrowed_lookup_preserves_overrides_order_and_missing_answers() {
        let local = "Pair(first, second) has\n    value first\n"
            .parse_source_full()
            .ast
            .tags;
        let name = Intern::from_ref("Pair");
        let mut package = local.clone();
        package
            .get_mut(&name)
            .unwrap()
            .params
            .as_mut()
            .unwrap()
            .swap_indices(0, 1);
        assert_eq!(local, package);
        let declarations = TagDeclarations::overlay(&local, &package);
        assert!(std::ptr::eq(
            declarations.get(&name).unwrap(),
            &local[&name]
        ));
        assert_eq!(
            declarations
                .get(&name)
                .unwrap()
                .params
                .as_ref()
                .unwrap()
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![Intern::from_ref("first"), Intern::from_ref("second")]
        );
        let empty = TagMap::new();
        assert!(std::ptr::eq(
            TagDeclarations::overlay(&empty, &package)
                .get(&name)
                .unwrap(),
            &package[&name]
        ));
    }

    #[test]
    fn domain_lookup_matches_local_first_cloning_through_aliases_and_edits() {
        let package = "Selected is 2 to 3\nFallback is Selected\n"
            .parse_source_full()
            .ast
            .tags;
        let types = HashMap::new();
        for source in [
            "Selected is 1 to 2\n",
            "Selected is 4 to 5\n",
            "Other is 5 to 6\n",
        ] {
            let local = source.parse_source_full().ast.tags;
            let declarations = TagDeclarations::overlay(&local, &package);
            let mut merged = package.clone();
            merged.extend(local.clone());
            for name in ["Selected", "Fallback", "Other", "Missing"] {
                let expr = ast::Expr::AnonymousTag(Intern::from_ref(name));
                let borrowed = crate::domain::Domains::new(
                    TypeEnv::new(&types).with_tag_declaration_lookup(declarations),
                    declarations,
                )
                .value(&expr);
                let cloned = crate::domain::Domains::new(
                    TypeEnv::new(&types).with_tag_decls(&merged),
                    &merged,
                )
                .value(&expr);
                assert_eq!(borrowed, cloned, "{source} / {name}");
                if name == "Fallback" {
                    assert!(borrowed.unwrap().is_some());
                }
            }
        }
    }
}
