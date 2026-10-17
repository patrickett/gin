use diagnostic::code::{ArrayCode, GenericCode, TraitCode, TypeCode};
use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::ops::ControlFlow;

use ast::{Bind, Expr, FileAst, NormalExpr, Ty};
use diagnostic::{Diagnostic, DiagnosticCode};
use internment::Intern;

use crate::typed::{BindBody, DefId, TypedBind, TypedExprKind, TypedFileAst, walk_expr_preorder};
use arguments::infer;
use substitute::{RewriteCall, specialize};

mod arguments;
mod constraints;
mod proof_getters;
mod queries;
mod substitute;
pub(super) use queries::SpecializationQueries;

type Methods = HashMap<(ast::ty::TypeId, Intern<String>), Intern<String>>;

fn visit_types(ty: &Ty, mut visit: impl FnMut(&Ty)) {
    let mut pending = vec![ty];
    while let Some(ty) = pending.pop() {
        visit(ty);
        let _ = ty.walk_children(&mut |child| {
            if let Some(ty) = child.ty() {
                pending.push(ty);
            }
            ControlFlow::<()>::Continue(())
        });
    }
}

impl TypedBind {
    pub fn type_parameters(&self) -> BTreeSet<Intern<String>> {
        assert_eq!(
            self.params.len(),
            self.param_kinds.len(),
            "specialization parameters and kinds must have equal lengths"
        );
        fn collect(ty: &Ty, names: &mut BTreeSet<Intern<String>>) {
            visit_types(ty, |ty| {
                if let Ty::Opaque(name) = ty {
                    names.insert(*name);
                }
            });
        }
        let mut names = self.body_type_parameters.as_ref().clone();
        if self.params.iter().any(|(name, _)| name.as_str() == "self")
            && let Some(receiver) = &self.receiver_type
        {
            collect(receiver, &mut names);
        }
        if self.is_callable {
            collect(&self.return_type, &mut names);
        }
        for ((name, ty), kind) in self.params.iter().zip(&self.param_kinds) {
            if matches!(kind, ast::ParamKind::Type) {
                names.insert(*name);
            } else {
                collect(ty, &mut names);
            }
        }
        names
    }

    pub fn requires_specialization(&self) -> bool {
        !self.array_length_parameters().is_empty() || !self.type_parameters().is_empty()
    }

    pub fn array_length_parameters(&self) -> BTreeSet<Intern<String>> {
        fn collect(ty: &Ty, names: &mut BTreeSet<Intern<String>>) {
            visit_types(ty, |ty| {
                if let Ty::Array {
                    size: NormalExpr::Var(name),
                    ..
                } = ty
                {
                    names.insert(*name);
                }
            });
        }
        let mut names = BTreeSet::new();
        for (_, ty) in self.params.iter() {
            collect(ty, &mut names);
        }
        names.retain(|name| !self.params.iter().any(|(param, _)| param == name));
        names
    }
}

pub(super) fn expand(files: &mut [FileAst], typed: &mut [TypedFileAst]) -> bool {
    expand_cached(files, typed, None).0
}

pub(super) fn expand_cached(
    files: &mut [FileAst],
    typed: &mut [TypedFileAst],
    queries: Option<&SpecializationQueries>,
) -> (bool, (usize, usize)) {
    let (expansion, work) = expand_pending(files, typed, queries);
    expansion.publish(typed);
    (expansion.expanded, work)
}

#[derive(Default)]
pub(super) struct Expansion {
    pub expanded: bool,
    diagnostics: Vec<(usize, crate::ExprId, Diagnostic)>,
}

impl Expansion {
    pub fn publish(&self, typed: &mut [TypedFileAst]) {
        for (file, call, diagnostic) in &self.diagnostics {
            typed[*file].exprs.facts.semantic_facts.flaws[call.as_usize()].push(diagnostic.clone());
        }
    }
}

pub(super) fn expand_pending(
    files: &mut [FileAst],
    typed: &[TypedFileAst],
    queries: Option<&SpecializationQueries>,
) -> (Expansion, (usize, usize)) {
    assert_eq!(
        files.len(),
        typed.len(),
        "specialization source and typed files must have equal lengths"
    );
    for (file, typed) in files.iter_mut().zip(typed.iter()) {
        *file = super::trait_methods::TraitMethods::new(file, &typed.eval_ast)
            .expand_defaults()
            .into_owned();
        inherit_payload_contracts(file, &typed.eval_ast);
    }
    let templates: HashMap<_, _> = typed
        .iter()
        .enumerate()
        .flat_map(|(file, typed)| {
            typed.defs.iter().filter_map(move |(id, bind)| {
                (bind.requires_specialization()).then_some((*id, (file, bind.clone())))
            })
        })
        .collect();
    if templates.is_empty() {
        return (Expansion::default(), (0, 0));
    }
    let declarations: ast::TagMap = if templates.values().any(|(_, template)| {
        template
            .param_constraints
            .iter()
            .any(|constraints| !constraints.is_empty())
    }) {
        typed
            .iter()
            .flat_map(|file| file.eval_ast.tags.iter())
            .chain(files.iter().flat_map(|file| file.tags.iter()))
            .map(|(name, declaration)| (*name, declaration.clone()))
            .collect()
    } else {
        ast::TagMap::new()
    };
    let methods: HashMap<_, _> = typed
        .iter()
        .flat_map(|file| file.defs.iter())
        .filter_map(|(id, bind)| {
            let receiver = bind.receiver_type.as_ref()?.named_instance()?.declaration;
            let member = Intern::from_ref(id.0.as_str().rsplit('.').next()?);
            Some(((receiver, member), id.0))
        })
        .collect();
    let environment = match queries {
        Some(queries) => queries.environment(queries::Environment::new(
            files,
            typed,
            &templates,
            &methods,
            declarations,
        )),
        None => std::sync::Arc::new(queries::Environment::uncached(
            typed,
            templates,
            methods,
            declarations,
        )),
    };
    let mut plans = Vec::with_capacity(typed.len());
    let mut work = (0, 0);
    let mut expansion = Expansion::default();
    for (file, typed) in typed.iter().enumerate() {
        let (plan, reused) = queries::plan(files, file, typed, &environment, queries);
        work.0 += usize::from(!reused);
        work.1 += usize::from(reused);
        for (call, diagnostic) in &plan.diagnostics {
            expansion
                .diagnostics
                .push((file, *call, diagnostic.clone()));
        }
        plans.push(plan);
    }
    let mut additions: Vec<(usize, DefId, Bind, Option<Bind>)> = Vec::new();
    for addition in plans
        .iter()
        .flat_map(|plan| &plan.proofs)
        .chain(plans.iter().flat_map(|plan| &plan.additions))
    {
        if !additions.iter().any(|(_, id, _, _)| id == &addition.1) {
            additions.push(addition.clone());
        }
    }
    let rewrites: Vec<_> = plans
        .into_iter()
        .enumerate()
        .flat_map(|(file, plan)| {
            plan.rewrites
                .into_iter()
                .map(move |(owner, span, id, template, member)| {
                    (file, owner, span, id, template, member)
                })
        })
        .collect();
    if rewrites.is_empty() && additions.is_empty() {
        return (expansion, work);
    }
    for (file, owner, span, definition, template, member_call) in rewrites {
        let file = &mut files[file];
        if let Some(bind) = file.defs.get_mut(&owner) {
            RewriteCall {
                spans: &file.span_table,
                span,
                definition,
                template: &template,
                member_call,
            }
            .rewrite_bind(bind);
        }
    }
    for (file, id, bind, method) in additions {
        files[file].private_defs.insert(bind.name);
        files[file].defs.insert(id, bind);
        if let Some(method) = method {
            files[file].method_binds.push(method);
        }
    }
    expansion.expanded = true;
    (expansion, work)
}

fn inherit_payload_contracts(file: &mut FileAst, package: &FileAst) {
    let traits = super::trait_methods::TraitMethods::new(file, package);
    let definitions: Vec<_> = file
        .defs
        .iter()
        .filter_map(|(id, bind)| {
            let effective = traits.effective(bind);
            (effective.payload_postconditions != bind.payload_postconditions)
                .then(|| (*id, effective.payload_postconditions.clone()))
        })
        .collect();
    let methods: Vec<_> = file
        .method_binds
        .iter()
        .enumerate()
        .filter_map(|(index, bind)| {
            let effective = traits.effective(bind);
            (effective.payload_postconditions != bind.payload_postconditions)
                .then(|| (index, effective.payload_postconditions.clone()))
        })
        .collect();
    for (id, proofs) in definitions {
        file.defs.get_mut(&id).unwrap().payload_postconditions = proofs;
    }
    for (index, proofs) in methods {
        file.method_binds[index].payload_postconditions = proofs;
    }
}

struct SpecializationCall {
    owner: DefId,
    expression: crate::ExprId,
    target: DefId,
    arguments: Vec<crate::ExprId>,
}

struct Specialization {
    definition: DefId,
    arguments: arguments::Arguments,
    category: &'static str,
    suffix: String,
}

fn plan_calls(
    files: &[FileAst],
    file: usize,
    typed: &TypedFileAst,
    templates: &HashMap<DefId, (usize, TypedBind)>,
    methods: &Methods,
    declarations: &ast::TagMap,
) -> queries::Plan {
    let mut plan = queries::Plan::default();
    for call in specialization_calls(typed, templates) {
        let (template_file, template) = &templates[&call.target];
        let specialization = match infer_specialization(
            &call,
            template,
            typed,
            files[file].defs.get(&call.owner),
            declarations,
        ) {
            Ok(Some(specialization)) => specialization,
            Ok(None) => continue,
            Err(diagnostic) => {
                plan.diagnostics.push((call.expression, diagnostic));
                continue;
            }
        };
        let id = specialization.definition;
        if !files[*template_file].defs.contains_key(&id)
            && !plan
                .additions
                .iter()
                .any(|(_, existing, _, _)| *existing == id)
        {
            let Some(addition) = specialization_addition(
                &files[*template_file],
                *template_file,
                call.target,
                &specialization,
                methods,
            ) else {
                continue;
            };
            plan.additions.push(addition);
        }
        plan.rewrites.push((
            call.owner,
            typed.exprs.span[call.expression.as_usize()],
            id,
            template.clone(),
            typed.resolved_member_calls.get(&call.expression).cloned(),
        ));
    }
    plan
}

fn specialization_calls(
    typed: &TypedFileAst,
    templates: &HashMap<DefId, (usize, TypedBind)>,
) -> Vec<SpecializationCall> {
    let mut calls = Vec::new();
    for (owner, bind) in &typed.defs {
        let roots = match &bind.body {
            BindBody::Expr(expr) => vec![*expr],
            BindBody::Body { exprs, ret } => exprs.iter().copied().chain(*ret).collect(),
            BindBody::Unlowered => continue,
        };
        for root in roots {
            let _ = walk_expr_preorder(typed, root, &mut |id| {
                if let Some(TypedExprKind::FnCall {
                    target,
                    args: Some(args),
                    ..
                }) = typed.exprs.kind_of(id)
                    && templates.contains_key(target)
                {
                    calls.push(SpecializationCall {
                        owner: *owner,
                        expression: id,
                        target: *target,
                        arguments: args.clone(),
                    });
                }
                ControlFlow::Continue(())
            });
        }
    }
    calls
}

fn infer_specialization(
    call: &SpecializationCall,
    template: &TypedBind,
    typed: &TypedFileAst,
    owner: Option<&Bind>,
    declarations: &ast::TagMap,
) -> Result<Option<Specialization>, Diagnostic> {
    let inferred =
        infer(template, &call.arguments, typed, call.expression, owner).map_err(|reason| {
            let code = if template.type_parameters().is_empty() {
                // type-array-length-inference
                DiagnosticCode::Type(TypeCode::Array(ArrayCode::LengthInference))
            } else {
                // type-generic-specialization
                DiagnosticCode::Type(TypeCode::Generic(GenericCode::Specialization))
            };
            Diagnostic::new(code, reason).at_span_id(
                typed.exprs.span[call.expression.as_usize()],
                &typed.span_table,
            )
        })?;
    let Some(arguments) = inferred else {
        return Ok(None);
    };
    constraints::validate(template, &arguments, typed, declarations).map_err(|reason| {
        // type-parameter-trait-constraint
        Diagnostic::new(TraitCode::ParameterConstraint, reason).at_span_id(
            typed.exprs.span[call.expression.as_usize()],
            &typed.span_table,
        )
    })?;
    let (category, suffix) = specialization_suffix(template, &arguments);
    let name = Intern::new(format!("{}${category}${suffix}", call.target.0));
    Ok(Some(Specialization {
        definition: call.target.with_name(name),
        arguments,
        category,
        suffix,
    }))
}

fn specialization_suffix(
    template: &TypedBind,
    arguments: &arguments::Arguments,
) -> (&'static str, String) {
    if template.type_parameters().is_empty() {
        let suffix = template
            .array_length_parameters()
            .iter()
            .map(|name| {
                let Expr::Lit(ast::Literal::Int(value)) = &arguments.values[name] else {
                    unreachable!()
                };
                value.to_string()
            })
            .collect::<Vec<_>>()
            .join("_");
        ("array", suffix)
    } else {
        let mut hasher = std::hash::DefaultHasher::new();
        arguments.values.hash(&mut hasher);
        arguments.receivers.hash(&mut hasher);
        ("specialized", format!("{:x}", hasher.finish()))
    }
}

fn specialization_addition(
    file: &FileAst,
    index: usize,
    target: DefId,
    specialization: &Specialization,
    methods: &Methods,
) -> Option<(usize, DefId, Bind, Option<Bind>)> {
    let parsed = file.defs.get(&target)?;
    let method = file
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
                Intern::new(format!(
                    "{}${}${}",
                    method.name, specialization.category, specialization.suffix
                )),
                &specialization.arguments,
                methods,
            )
        });
    Some((
        index,
        specialization.definition,
        specialize(
            parsed,
            specialization.definition.0,
            &specialization.arguments,
            methods,
        ),
        method,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FileId;
    use parser::query::SourceParseExt;

    fn value_template(ty: Ty) -> TypedBind {
        let parsed = "identity(value): return value\n".parse_source_full();
        let typed = crate::transform::stage_declare(
            &parsed.ast,
            FileId(0),
            &crate::transform::TransformCtx::new(),
        );
        let mut bind = typed.defs.values().next().unwrap().clone();
        let mut params = bind.params.to_vec();
        params[0].1 = ty.clone();
        bind.params = params.into();
        bind.param_kinds[0] = ast::ParamKind::Value(Box::new(ty));
        bind.return_type = Ty::Unit;
        std::sync::Arc::make_mut(&mut bind.body_type_parameters).clear();
        bind
    }

    #[test]
    fn type_parameter_discovery_visits_storage_and_record_fields() {
        let element = Intern::from_ref("element");
        let field = Intern::from_ref("field");
        for ty in [
            Ty::OwnedStorage {
                inner: Box::new(Ty::Opaque(element)),
            },
            Ty::Record {
                name: Intern::from_ref("Record"),
                fields: vec![(field, Box::new(Ty::Opaque(element)))],
                resolved_params: None,
            },
            Ty::sequence(vec![ast::ty::SequenceTypeElement {
                name: Some(field),
                ty: Box::new(Ty::Opaque(element)),
            }]),
        ] {
            let bind = value_template(ty);
            assert_eq!(bind.type_parameters(), BTreeSet::from([element]));
            assert!(bind.requires_specialization());
        }
    }

    #[test]
    fn array_length_discovery_visits_storage_and_record_fields() {
        let length = Intern::from_ref("length");
        let array = Ty::Array {
            elem: Box::new(Ty::Unit),
            size: NormalExpr::Var(length),
        };
        for ty in [
            Ty::OwnedStorage {
                inner: Box::new(array.clone()),
            },
            Ty::Record {
                name: Intern::from_ref("Record"),
                fields: vec![(Intern::from_ref("field"), Box::new(array))],
                resolved_params: None,
            },
        ] {
            let bind = value_template(ty);
            assert_eq!(bind.array_length_parameters(), BTreeSet::from([length]));
            assert!(bind.requires_specialization());
        }
        let mut explicit = value_template(Ty::Array {
            elem: Box::new(Ty::Unit),
            size: NormalExpr::Var(Intern::from_ref("value")),
        });
        assert!(explicit.array_length_parameters().is_empty());
        let mut params = explicit.params.to_vec();
        params[0].1 = Ty::Unit;
        explicit.params = params.into();
        assert!(explicit.type_parameters().is_empty());
    }

    #[test]
    fn expansion_rejects_unpaired_source_and_typed_files() {
        for (source_count, typed_count) in [(1, 0), (0, 1)] {
            let mut files = vec![FileAst::default(); source_count];
            let mut typed: Vec<_> = (0..typed_count)
                .map(|id| TypedFileAst::new(FileId(id), ast::SpanTable::new()))
                .collect();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                expand(&mut files, &mut typed);
            }));
            let error = result.expect_err("unpaired specialization inputs must be rejected");
            assert!(
                error
                    .downcast_ref::<String>()
                    .unwrap()
                    .contains("specialization source and typed files must have equal lengths")
            );
        }
    }

    #[test]
    fn expansion_accepts_an_empty_package() {
        assert!(!expand(&mut [], &mut []));
    }

    #[test]
    fn type_parameter_discovery_rejects_unpaired_parameter_metadata() {
        let parsed = "identity(value): return value\n".parse_source_full();
        let typed = crate::transform::stage_declare(
            &parsed.ast,
            FileId(0),
            &crate::transform::TransformCtx::new(),
        );
        let original = typed.defs.values().next().unwrap();
        assert_eq!(original.params.len(), 1);
        assert_eq!(original.param_kinds.len(), 1);
        original.type_parameters();
        for kinds in [
            smallvec::smallvec![],
            smallvec::smallvec![ast::ParamKind::Type; 2],
        ] {
            let mut malformed = original.clone();
            malformed.param_kinds = kinds;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                malformed.type_parameters();
            }));
            let error = result.expect_err("unpaired parameter metadata must be rejected");
            assert!(
                error
                    .downcast_ref::<String>()
                    .unwrap()
                    .contains("specialization parameters and kinds must have equal lengths")
            );
        }
    }
}
