use std::cell::RefCell;

use super::*;
use crate::analysis::TypeEnv;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct AliasRead {
    pub name: Intern<String>,
    pub lookup: Lookup,
    pub answer: Option<Ty>,
    pub parameters: Option<Vec<(Intern<String>, ast::Parameter)>>,
}

struct Probe {
    answer: Ty,
    reads: Vec<AliasRead>,
    pending: Vec<TypeId>,
    deferred: bool,
}

impl DeclarationCatalog<'_> {
    fn query_alias(&self, identity: TypeId) -> Option<&Expr> {
        let declaration = self.declarations[&identity].value;
        if declare::nominal_type(declaration, FileId(identity.file)).is_some()
            || declaration.attributes.fixed_array.is_some()
            || declaration.params.as_ref().is_some_and(|params| {
                params.contains_key(&declaration.name)
                    || params.values().any(|param| {
                        !matches!(param.kind, ParameterKind::Generic)
                            || param.range_pattern.is_some()
                    })
            })
        {
            return None;
        }
        match &declaration.value {
            DeclareValue::Alias(alias) if self.query_surface(identity.file, &alias.value) => {
                Some(&alias.value)
            }
            _ => None,
        }
    }

    fn query_surface(&self, scope: u32, expr: &Expr) -> bool {
        match expr {
            Expr::AnonymousTag(_) => true,
            Expr::FnCall(call) => call.args.is_none() && call.path.value.segments.is_empty(),
            Expr::Ref { inner, .. } => self.query_surface(scope, &inner.value),
            Expr::TagCall(call) => {
                let Lookup::Found(identity) = self.lookup(scope, call.name) else {
                    return false;
                };
                let declaration = self.declarations[&identity].value;
                matches!(declaration.value, DeclareValue::Alias(_))
                    && declare::nominal_type(declaration, FileId(identity.file)).is_none()
                    && declaration.attributes.fixed_array.is_none()
                    && !call.args.is_empty()
                    && declaration.params.as_ref().is_some_and(|params| {
                        params.len() == call.args.len()
                            && params.values().all(|param| {
                                matches!(param.kind, ParameterKind::Generic)
                                    && param.range_pattern.is_none()
                            })
                    })
                    && call
                        .args
                        .iter()
                        .all(|arg| self.query_surface(scope, &arg.value))
            }
            _ => false,
        }
    }

    fn probe_alias(&self, identity: TypeId, resolved: &HashMap<TypeId, Option<Ty>>) -> Probe {
        let surface = self.query_alias(identity).unwrap();
        let mut surface_names = HashSet::new();
        query_surface_names(surface, &mut surface_names);
        let mut types = HashMap::new();
        let mut params = HashMap::new();
        for name in &surface_names {
            let Lookup::Found(target) = self.lookup(identity.file, *name) else {
                continue;
            };
            let declaration = self.declarations[&target].value;
            if let Some(ty) = resolved
                .get(&target)
                .and_then(Option::as_ref)
                .cloned()
                .or_else(|| declare::nominal_type(declaration, FileId(target.file)))
            {
                types.insert(*name, ty);
            }
            if let Some(generic) = &declaration.params
                && generic
                    .values()
                    .all(|param| matches!(param.kind, ParameterKind::Generic))
            {
                params.insert(*name, generic.clone());
            }
        }
        let subst: HashMap<_, _> = self.declarations[&identity]
            .value
            .params
            .iter()
            .flat_map(|params| params.keys())
            .map(|name| (*name, Ty::Opaque(*name)))
            .collect();
        let names = RefCell::new(HashSet::new());
        let answer = TypeEnv::new(&types)
            .with_tag_params(&params)
            .with_subst(&subst)
            .with_type_reads(&names)
            .resolve_expr(surface);
        let mut reads = Vec::new();
        let mut pending = Vec::new();
        let mut deferred = false;
        for name in names.into_inner() {
            let lookup = self.lookup(identity.file, name);
            if let Lookup::Found(target) = &lookup {
                let declaration = self.declarations[target].value;
                if declaration.params.as_ref().is_some_and(|params| {
                    params
                        .values()
                        .any(|param| !matches!(param.kind, ParameterKind::Generic))
                }) {
                    deferred = true;
                } else if declare::nominal_type(declaration, FileId(target.file)).is_none() {
                    match resolved.get(target) {
                        Some(Some(_)) => {}
                        Some(None) => deferred = true,
                        None if self.query_alias(*target).is_some() => pending.push(*target),
                        None => deferred = true,
                    }
                }
            } else if matches!(lookup, Lookup::Ambiguous(_)) {
                deferred = true;
            }
            let parameters = match &lookup {
                Lookup::Found(target) => {
                    self.declarations[target]
                        .value
                        .params
                        .as_ref()
                        .map(|params| {
                            params
                                .iter()
                                .map(|(name, parameter)| (*name, parameter.clone()))
                                .collect()
                        })
                }
                _ => None,
            };
            reads.push(AliasRead {
                name,
                lookup,
                answer: types.get(&name).cloned(),
                parameters,
            });
        }
        reads.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
        pending.sort_by(|left, right| {
            (left.file, left.name.as_str()).cmp(&(right.file, right.name.as_str()))
        });
        Probe {
            answer,
            reads,
            pending,
            deferred,
        }
    }

    pub(super) fn resolve_alias_queries(&self, aliases: &mut ResolvedAliases) {
        let mut resolved: HashMap<_, _> = self
            .order
            .iter()
            .filter_map(|identity| {
                (self.candidates[&identity.name].len() == 1)
                    .then(|| {
                        aliases
                            .types
                            .get(&TagId(identity.name))
                            .cloned()
                            .map(|answer| (*identity, Some(answer)))
                    })
                    .flatten()
            })
            .collect();
        for root in &self.order {
            if resolved.contains_key(root) || self.query_alias(*root).is_none() {
                continue;
            }
            let mut stack = vec![*root];
            let mut active = HashSet::from([*root]);
            while let Some(identity) = stack.last().copied() {
                let probe = self.probe_alias(identity, &resolved);
                let invalid_dependency = probe.reads.iter().any(|read| {
                    matches!(read.lookup, Lookup::Found(target) if aliases.invalid.contains(&target))
                });
                aliases.reads.insert(identity, probe.reads);
                if let Some(dependency) = probe.pending.first().copied()
                    && !probe.deferred
                    && !invalid_dependency
                {
                    if active.insert(dependency) {
                        stack.push(dependency);
                        continue;
                    }
                    for identity in stack.drain(..) {
                        resolved.insert(identity, None);
                    }
                    active.clear();
                    continue;
                }
                let answer = (!probe.deferred && !invalid_dependency).then_some(probe.answer);
                resolved.insert(identity, answer);
                stack.pop();
                active.remove(&identity);
            }
        }
        for (identity, answer) in resolved {
            if self.candidates[&identity.name].len() == 1
                && let Some(answer) = answer
            {
                aliases.types.insert(TagId(identity.name), answer);
            }
        }
    }
}

fn query_surface_names(expr: &Expr, names: &mut HashSet<Intern<String>>) {
    match expr {
        Expr::AnonymousTag(name) => {
            names.insert(*name);
        }
        Expr::FnCall(call) => {
            names.insert(call.path.value.root);
        }
        Expr::Ref { inner, .. } => query_surface_names(&inner.value, names),
        Expr::TagCall(call) => {
            names.insert(call.name);
            for arg in &call.args {
                query_surface_names(&arg.value, names);
            }
        }
        _ => unreachable!("only bounded alias surfaces are probed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::query::SourceParseExt;

    #[test]
    fn alias_reads_include_absence_and_exclude_binders() {
        let sources = ["A is B(Missing)\n", "B(element) is ref element\n"];
        let files: Vec<_> = sources
            .iter()
            .enumerate()
            .map(|(index, source)| {
                let mut parsed = source.parse_source_full();
                crate::prepare_parse_ast(&mut parsed.ast, &flask::CompileTarget::Library);
                (parsed.ast, FileId(index as u32))
            })
            .collect();
        let catalog = DeclarationCatalog::collect(&files);
        let aliases = catalog.resolve_named_aliases();
        let id = TypeId {
            file: 0,
            name: Intern::from_ref("A"),
        };
        let reads = &aliases.reads[&id];
        assert!(reads.iter().any(|read| read.name.as_str() == "Missing"
            && matches!(read.lookup, Lookup::Missing(_))
            && read.answer.is_none()));
        assert!(reads.iter().any(|read| {
            read.name.as_str() == "B"
                && matches!(read.lookup, Lookup::Found(_))
                && matches!(read.answer, Some(Ty::Ref { .. }))
                && read
                    .parameters
                    .as_ref()
                    .is_some_and(|params| params.iter().any(|(name, _)| name.as_str() == "element"))
        }));
        assert!(
            aliases.reads[&TypeId {
                file: 1,
                name: Intern::from_ref("B")
            }]
                .is_empty()
        );
    }
    #[test]
    fn ambiguous_alias_reads_record_the_completed_candidate_set() {
        let files: Vec<_> = ["A is ref Target\n", "Target is Word\n", "Target is Word\n"]
            .iter()
            .enumerate()
            .map(|(index, source)| (source.parse_source_full().ast, FileId(index as u32)))
            .collect();
        let catalog = DeclarationCatalog::collect(&files);
        let aliases = catalog.resolve_named_aliases();
        let id = TypeId {
            file: 0,
            name: Intern::from_ref("A"),
        };
        assert!(!aliases.types.contains_key(&TagId(id.name)));
        assert!(aliases.reads[&id].iter().any(
            |read| matches!(&read.lookup, Lookup::Ambiguous(candidates) if candidates.len() == 2)
        ));
    }
    #[test]
    fn aliases_with_domain_parameter_evaluation_are_deferred() {
        let mut file = "B(element) is ref element\n".parse_source_full().ast;
        let name = Intern::from_ref("B");
        let declaration = file.tags.get_mut(&name).unwrap();
        let parameter = declaration
            .params
            .as_mut()
            .unwrap()
            .values_mut()
            .next()
            .unwrap();
        parameter.range_pattern = Some(RangeParameterPattern {
            start: Intern::from_ref("lower"),
            end: Intern::from_ref("upper"),
            end_span: parameter.name_span,
        });
        let files = vec![(file, FileId(0))];
        let catalog = DeclarationCatalog::collect(&files);
        let aliases = catalog.resolve_named_aliases();
        assert!(!aliases.types.contains_key(&TagId(name)));
        assert!(!aliases.reads.contains_key(&TypeId { file: 0, name }));
    }
    #[test]
    fn generic_alias_cycles_are_deferred_without_speculative_diagnostics() {
        let files: Vec<_> = ["A(element) is B(element)\n", "B(element) is A(element)\n"]
            .iter()
            .enumerate()
            .map(|(index, source)| (source.parse_source_full().ast, FileId(index as u32)))
            .collect();
        let aliases = DeclarationCatalog::collect(&files).resolve_named_aliases();
        assert!(aliases.types.is_empty());
        assert!(aliases.invalid.is_empty());
        assert_eq!(aliases.reads.len(), 2);
    }
    #[test]
    fn parameter_order_is_part_of_an_alias_dependency_answer() {
        let resolve = |template: &str| {
            let files: Vec<_> = [
                "A is B(Word, Other)\nWord is in 0 to 255\nOther is in 0 to 65535\n",
                template,
            ]
            .iter()
            .enumerate()
            .map(|(index, source)| (source.parse_source_full().ast, FileId(index as u32)))
            .collect();
            DeclarationCatalog::collect(&files).resolve_named_aliases()
        };
        let before = resolve("B(first, second) is ref first\n");
        let after = resolve("B(second, first) is ref first\n");
        let id = TypeId {
            file: 0,
            name: Intern::from_ref("A"),
        };
        let dependency = |aliases: &ResolvedAliases| {
            aliases.reads[&id]
                .iter()
                .find(|read| read.name.as_str() == "B")
                .unwrap()
                .clone()
        };
        let old = dependency(&before);
        let new = dependency(&after);
        assert_eq!(old.answer, new.answer);
        assert_ne!(old.parameters, new.parameters);
        assert_ne!(before.types[&TagId(id.name)], after.types[&TagId(id.name)]);
    }
}
