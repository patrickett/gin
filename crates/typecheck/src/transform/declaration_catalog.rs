use diagnostic::DiagnosticCode;
use diagnostic::code::RepresentationCode;
use std::collections::{HashMap, HashSet};

use ast::prelude::*;
use ast::ty::TypeId;
use ast::{Declare, DeclareValue, FileAst, Ty};

use super::{TransformCtx, declare};
use crate::typed::{FileId, TagId, TypedFileAst};

mod alias_queries;

struct Declaration<'a> {
    file: &'a FileAst,
    value: &'a Declare,
}

#[derive(Clone, Debug, PartialEq)]
enum Lookup {
    Found(TypeId),
    Missing(Intern<String>),
    Ambiguous(Vec<TypeId>),
}

#[derive(Clone)]
enum AliasInput {
    Signature(TypeId),
    Identity(Ty),
    Missing(Intern<String>),
    Deferred,
}

pub(super) struct DeclarationCatalog<'a> {
    declarations: HashMap<TypeId, Declaration<'a>>,
    candidates: HashMap<Intern<String>, Vec<TypeId>>,
    order: Vec<TypeId>,
    nominal_identities: Vec<(TagId, Ty)>,
}

pub(super) struct ResolvedAliases {
    types: HashMap<TagId, Ty>,
    invalid: HashSet<TypeId>,
    reads: HashMap<TypeId, Vec<alias_queries::AliasRead>>,
}

impl<'a> DeclarationCatalog<'a> {
    pub fn collect(files: &'a [(FileAst, FileId)]) -> Self {
        assert_eq!(
            files.iter().map(|(_, id)| id).collect::<HashSet<_>>().len(),
            files.len(),
            "file identities must be unique"
        );
        let mut declarations = HashMap::new();
        let mut candidates: HashMap<_, Vec<_>> = HashMap::new();
        let mut nominal_identities = Vec::new();
        for (file, file_id) in files {
            for (name, value) in &file.tags {
                let identity = TypeId {
                    file: file_id.0,
                    name: *name,
                };
                if let Some(ty) = declare::nominal_type(value, *file_id) {
                    nominal_identities.push((TagId(*name), ty));
                }
                declarations.insert(identity, Declaration { file, value });
                candidates.entry(*name).or_default().push(identity);
            }
        }
        let mut order: Vec<_> = declarations.keys().copied().collect();
        order.sort_by(|left, right| {
            (left.file, left.name.as_str()).cmp(&(right.file, right.name.as_str()))
        });
        for identities in candidates.values_mut() {
            identities.sort_by_key(|identity| identity.file);
        }
        Self {
            declarations,
            candidates,
            order,
            nominal_identities,
        }
    }

    fn lookup(&self, scope: u32, name: Intern<String>) -> Lookup {
        let local = TypeId { file: scope, name };
        if self.declarations.contains_key(&local) {
            return Lookup::Found(local);
        }
        match self.candidates.get(&name).map(Vec::as_slice) {
            Some([identity]) => Lookup::Found(*identity),
            None => Lookup::Missing(name),
            Some(identities) => Lookup::Ambiguous(identities.to_vec()),
        }
    }

    pub fn seed_identities(&self, ctx: &mut TransformCtx) {
        ctx.cross_file_tag_types
            .extend(self.nominal_identities.clone());
    }

    fn alias_input(&self, identity: TypeId) -> AliasInput {
        let declaration = &self.declarations[&identity];
        let Some(name) = named_alias(declaration.value) else {
            return AliasInput::Deferred;
        };
        let target = match self.lookup(identity.file, name) {
            Lookup::Found(target) => target,
            Lookup::Missing(name) => return AliasInput::Missing(name),
            Lookup::Ambiguous(_) => return AliasInput::Deferred,
        };
        let target_declaration = self.declarations[&target].value;
        if target_declaration.params.as_ref().is_some_and(|params| {
            !params.is_empty()
                && params
                    .values()
                    .all(|param| matches!(param.kind, ParameterKind::Default(_)))
        }) {
            return AliasInput::Deferred;
        }
        if let Some(ty) = declare::nominal_type(target_declaration, FileId(target.file)) {
            AliasInput::Identity(ty)
        } else if named_alias(target_declaration).is_some() {
            AliasInput::Signature(target)
        } else {
            AliasInput::Deferred
        }
    }

    pub fn resolve_named_aliases(&self) -> ResolvedAliases {
        let inputs: HashMap<_, _> = self
            .order
            .iter()
            .filter(|identity| named_alias(self.declarations[identity].value).is_some())
            .map(|identity| (*identity, self.alias_input(*identity)))
            .collect();
        let mut resolved: HashMap<TypeId, Option<Ty>> = HashMap::new();
        let mut invalid = HashSet::new();
        for root in &self.order {
            if !inputs.contains_key(root) || resolved.contains_key(root) {
                continue;
            }
            let mut path = Vec::new();
            let mut active = HashSet::new();
            let mut next = *root;
            let answer = loop {
                if let Some(answer) = resolved.get(&next) {
                    if invalid.contains(&next) {
                        invalid.extend(path.iter().copied());
                    }
                    break answer.clone();
                }
                if !active.insert(next) {
                    invalid.extend(path.iter().copied());
                    break None;
                }
                path.push(next);
                match &inputs[&next] {
                    AliasInput::Signature(dependency) => next = *dependency,
                    AliasInput::Identity(ty) => break Some(ty.clone()),
                    AliasInput::Missing(name) => break Some(Ty::Opaque(*name)),
                    AliasInput::Deferred => break None,
                }
            };
            for identity in path {
                let answer = if invalid.contains(&identity) {
                    Some(Ty::Opaque(identity.name))
                } else {
                    answer.clone()
                };
                resolved.insert(identity, answer);
            }
        }
        let types = resolved
            .into_iter()
            .filter_map(|(identity, ty)| {
                if self.candidates[&identity.name].len() == 1 {
                    ty.map(|ty| (TagId(identity.name), ty))
                } else {
                    None
                }
            })
            .collect();
        let mut aliases = ResolvedAliases {
            types,
            invalid,
            reads: HashMap::new(),
        };
        self.resolve_alias_queries(&mut aliases);
        aliases
    }

    pub fn publish_diagnostics(&self, aliases: &ResolvedAliases, typed: &mut TypedFileAst) {
        for identity in &self.order {
            if identity.file != typed.file_id.0 || !aliases.invalid.contains(identity) {
                continue;
            }
            let declaration = &self.declarations[identity];
            let span = declaration.value.name_span;
            if typed.declaration_flaws.iter().any(|(existing_span, flaw)| {
                *existing_span == span
                    && flaw.code == DiagnosticCode::Representation(RepresentationCode::Recursive)
            }) {
                continue;
            }
            typed.declaration_flaws.push((
                span,
                crate::representation::RepresentationError::RecursiveRepresentation {
                    type_id: *identity,
                }
                .diagnostic()
                .with_arg("declaration", identity.name.as_str().to_string())
                .with_arg("type_name", identity.name.as_str().to_string())
                .at_span_id(span, &declaration.file.span_table),
            ));
        }
    }
}

impl ResolvedAliases {
    pub fn completed_types(&self) -> HashMap<TagId, Ty> {
        let invalid_names: HashSet<_> = self.invalid.iter().map(|identity| identity.name).collect();
        self.types
            .iter()
            .filter(|(id, _)| !invalid_names.contains(&id.0))
            .map(|(id, ty)| (*id, ty.clone()))
            .collect()
    }

    pub fn publish(&self, ctx: &mut TransformCtx) {
        ctx.cross_file_tag_types.extend(self.types.clone());
    }
}

fn named_alias(declaration: &Declare) -> Option<Intern<String>> {
    if declaration
        .params
        .as_ref()
        .is_some_and(|params| !params.is_empty())
    {
        return None;
    }
    let DeclareValue::Alias(alias) = &declaration.value else {
        return None;
    };
    let name = match &alias.value {
        Expr::AnonymousTag(name) => *name,
        Expr::FnCall(call) if call.args.is_none() && call.path.value.segments.is_empty() => {
            call.path.value.root
        }
        _ => return None,
    };
    name.as_str()
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_uppercase())
        .then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::query::SourceParseExt;

    #[test]
    #[should_panic(expected = "file identities must be unique")]
    fn duplicate_file_identities_are_rejected_even_without_declarations() {
        let files = [
            (FileAst::default(), FileId(7)),
            (FileAst::default(), FileId(7)),
        ];
        DeclarationCatalog::collect(&files);
    }

    #[test]
    #[should_panic(expected = "file identities must be unique")]
    fn duplicate_file_identities_cannot_overwrite_declarations() {
        let files = [
            ("Value is True\n".parse_source_full().ast, FileId(7)),
            ("Value is False\n".parse_source_full().ast, FileId(7)),
        ];
        DeclarationCatalog::collect(&files);
    }

    #[test]
    #[should_panic(expected = "file identities must be unique")]
    fn uncached_package_transform_rejects_duplicate_file_identities() {
        let files = [
            (FileAst::default(), FileId(7)),
            (FileAst::default(), FileId(7)),
        ];
        crate::transform::transform_package(
            &files,
            &TransformCtx::new(),
            crate::transform::PackageTransformOptions::default(),
        );
    }

    #[test]
    fn same_name_in_distinct_files_preserves_local_lookup_and_ambiguity() {
        let files = [
            ("Value is True\n".parse_source_full().ast, FileId(7)),
            ("Value is False\n".parse_source_full().ast, FileId(3)),
        ];
        let catalog = DeclarationCatalog::collect(&files);
        let name = Intern::from_ref("Value");
        let first = TypeId { file: 7, name };
        let second = TypeId { file: 3, name };
        assert_eq!(catalog.order, vec![second, first]);
        assert_eq!(catalog.lookup(7, name), Lookup::Found(first));
        assert_eq!(catalog.lookup(3, name), Lookup::Found(second));
        assert_eq!(
            catalog.lookup(0, name),
            Lookup::Ambiguous(vec![second, first])
        );
    }
}
