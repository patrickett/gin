use super::*;

pub(super) fn expand(
    files: &[FileAst],
    typed: &[TypedFileAst],
    templates: &HashMap<DefId, (usize, TypedBind)>,
    methods: &Methods,
    additions: &mut Vec<(usize, DefId, Bind, Option<Bind>)>,
    proof_members: &std::collections::HashSet<Intern<String>>,
) {
    let mut targets: Vec<_> = templates.iter().collect();
    targets.sort_by_key(|(id, _)| (id.0.as_str(), id.1));
    for file in typed.iter() {
        for receiver in file.exprs.ty.iter().chain(
            file.defs
                .values()
                .flat_map(|bind| bind.params.iter().map(|(_, ty)| ty)),
        ) {
            let Some(instance) = receiver.named_instance_stripping_reference_wrappers() else {
                continue;
            };
            for (target, (template_file, template)) in targets.iter().copied() {
                let member = target.0.as_str().rsplit('.').next().unwrap_or("");
                if !proof_members.contains(&Intern::from_ref(member))
                    || template.params.len() != 1
                    || template.param_conventions.first() != Some(&ast::ParamConvention::Observe)
                    || !template.params[0]
                        .1
                        .named_instance_stripping_reference_wrappers()
                        .is_some_and(|owner| owner.declaration == instance.declaration)
                {
                    continue;
                }
                let Some(arguments) = arguments::for_receiver(template, receiver, file) else {
                    continue;
                };
                let mut hasher = std::hash::DefaultHasher::new();
                arguments.values.hash(&mut hasher);
                arguments.receivers.hash(&mut hasher);
                let suffix = format!("{:x}", hasher.finish());
                let name = Intern::new(format!("{}$specialized${suffix}", target.0));
                let id = target.with_name(name);
                if files[*template_file].defs.contains_key(&id)
                    || additions.iter().any(
                        |(_, existing, _, _): &(usize, DefId, Bind, Option<Bind>)| *existing == id,
                    )
                {
                    continue;
                }
                let Some(parsed) = files[*template_file].defs.get(target) else {
                    continue;
                };
                let method = files[*template_file]
                    .method_binds
                    .iter()
                    .find(|method| {
                        method.name_span == parsed.name_span
                            && (parsed.name == method.name
                                || parsed.name.as_str().ends_with(&format!(".{}", method.name)))
                    })
                    .map(|method| {
                        specialize(
                            method,
                            Intern::new(format!("{}$specialized${suffix}", method.name)),
                            &arguments,
                            methods,
                        )
                    });
                additions.push((
                    *template_file,
                    id,
                    specialize(parsed, name, &arguments, methods),
                    method,
                ));
            }
        }
    }
}

pub(super) fn members(typed: &[TypedFileAst]) -> std::collections::HashSet<Intern<String>> {
    typed
        .iter()
        .flat_map(|file| {
            file.defs
                .values()
                .flat_map(|bind| {
                    let mut refinements: Vec<_> =
                        bind.param_refinements.iter().flatten().cloned().collect();
                    refinements.extend(bind.payload_postconditions.iter().map(|proof| {
                        ast::PredicateExpr::Proposition(Box::new(proof.proposition.clone()))
                    }));
                    if let Some(proof) = &bind.postcondition {
                        refinements.push(ast::PredicateExpr::Proposition(Box::new(
                            proof.proposition.clone(),
                        )));
                    }
                    refinements
                })
                .chain(
                    file.tags
                        .values()
                        .flat_map(|tag| tag.record_field_refinements.values().cloned()),
                )
                .flat_map(|predicate| super::super::refinement_terms::predicate_names(&predicate))
                .flat_map(|name| {
                    name.as_str()
                        .split('.')
                        .skip(1)
                        .map(Intern::from_ref)
                        .collect::<Vec<_>>()
                })
        })
        .collect()
}
