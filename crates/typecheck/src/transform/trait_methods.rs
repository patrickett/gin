use ast::folder::{Folder, ParameterSubstitution};
use diagnostic::code::{TraitCode, TypeCode};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use ast::{Bind, Declare, DeclareValue, Expr, FileAst, HasFunction, HasMember, ParameterKind};
use diagnostic::{Diagnostic, DiagnosticCode};
use internment::Intern;

use super::proof_walk::write::{ProofVisitorMut, walk_term};

pub(super) struct TraitMethods<'a> {
    file: &'a FileAst,
    package: &'a FileAst,
    reads: Option<&'a super::implementation_reads::ImplementationReadRecorder>,
}

#[derive(Clone)]
pub(super) struct MethodContract {
    pub owner: Intern<String>,
    pub method: HasFunction,
}

impl<'a> TraitMethods<'a> {
    pub fn new(file: &'a FileAst, package: &'a FileAst) -> Self {
        Self {
            file,
            package,
            reads: None,
        }
    }

    pub fn with_reads(
        mut self,
        reads: Option<&'a super::implementation_reads::ImplementationReadRecorder>,
    ) -> Self {
        self.reads = reads;
        self
    }

    fn declaration(&self, name: Intern<String>) -> Option<&'a Declare> {
        if let Some(reads) = self.reads {
            reads.trait_declaration(name);
        }
        self.file
            .tags
            .get(&name)
            .or_else(|| self.package.tags.get(&name))
    }

    fn collect(
        &self,
        name: Intern<String>,
        args: &[ast::Typed<Expr>],
        seen: &mut HashSet<Intern<String>>,
        methods: &mut Vec<MethodContract>,
    ) {
        if !seen.insert(name) {
            return;
        }
        let Some(decl) = self.declaration(name) else {
            return;
        };
        let subst: HashMap<_, _> = decl
            .params
            .iter()
            .flat_map(|p| p.keys())
            .zip(args)
            .map(|(name, arg)| (*name, arg.value.clone()))
            .collect();
        if let DeclareValue::Has(members) = &decl.value {
            for member in members {
                if let HasMember::Function(method) = member {
                    let method = specialize_method(method, &subst);
                    if !methods
                        .iter()
                        .any(|c| c.owner == name && c.method == method)
                    {
                        methods.push(MethodContract {
                            owner: name,
                            method,
                        });
                    }
                }
            }
        }
        for provided in &decl.provided_traits {
            let mut args = provided.args.clone();
            for arg in &mut args {
                let _ = ParameterSubstitution::new(&subst).visit_expr(&mut arg.value);
            }
            self.collect(provided.trait_name, &args, seen, methods);
        }
        seen.remove(&name);
    }

    pub fn required(&self, decl: &Declare) -> Vec<MethodContract> {
        let mut methods = Vec::new();
        let mut seen = HashSet::new();
        for provided in &decl.provided_traits {
            self.collect(provided.trait_name, &provided.args, &mut seen, &mut methods);
        }
        methods
    }

    pub fn unimplemented(&self, decl: &Declare) -> Vec<MethodContract> {
        if let Some(reads) = self.reads {
            reads.trait_candidates();
        }
        let mut contracts = self.required(decl);
        if let DeclareValue::Has(members) = &decl.value {
            contracts.extend(members.iter().filter_map(|member| match member {
                HasMember::Function(method) => Some(MethodContract {
                    owner: decl.name,
                    method: method.as_ref().clone(),
                }),
                _ => None,
            }));
        }
        contracts
            .iter()
            .filter(|contract| {
                contract.method.body.is_none()
                    && !contracts.iter().any(|candidate| {
                        candidate.method.name == contract.method.name
                            && candidate.method.body.is_some()
                    })
                    && !self
                        .file
                        .defs
                        .values()
                        .chain(self.package.defs.values())
                        .any(|bind| {
                            self.receiver(bind)
                                .is_some_and(|receiver| receiver.name == decl.name)
                                && !matches!(bind.value, ast::BindValue::Unassigned)
                                && (self.member_name(bind) == contract.method.name.as_str()
                                    || self.contracts(bind).iter().any(|candidate| {
                                        candidate.owner == contract.owner
                                            && candidate.method.name == contract.method.name
                                    }))
                        })
            })
            .cloned()
            .collect()
    }

    pub fn is_interface(&self, decl: &Declare) -> bool {
        matches!(&decl.value, DeclareValue::Has(members)
            if !members.iter().any(|member| matches!(member, HasMember::Property(_))))
            && !self.unimplemented(decl).is_empty()
    }

    fn receiver(&self, bind: &Bind) -> Option<&'a Declare> {
        let receiver = bind.receiver_type_surface()?;
        let name = match &receiver.value {
            Expr::AnonymousTag(name) => *name,
            Expr::TagCall(call) => call.name,
            _ => return None,
        };
        self.declaration(name)
    }

    fn member_name<'b>(&'b self, bind: &'b Bind) -> &'b str {
        let name = self
            .receiver(bind)
            .and_then(|receiver| {
                bind.name
                    .as_str()
                    .strip_prefix(&format!("{}.", receiver.name))
                    .or_else(|| {
                        bind.name
                            .as_str()
                            .rsplit_once(&format!(".{}.", receiver.name))
                            .map(|(_, member)| member)
                    })
            })
            .unwrap_or(bind.name.as_str());
        self.file
            .method_binds
            .iter()
            .find(|method| {
                method.name_span == bind.name_span
                    && (method.name == bind.name
                        || bind.name.as_str().ends_with(&format!(".{}", method.name)))
            })
            .map_or(name, |method| method.name.as_str())
    }

    pub fn contracts(&self, bind: &Bind) -> Vec<MethodContract> {
        let Some(decl) = self.receiver(bind) else {
            return Vec::new();
        };
        let member = self.member_name(bind);
        let (qualifier, name) = member
            .split_once('.')
            .map_or((None, member), |(q, n)| (Some(q), n));
        let mut contracts = self.required(decl);
        if let Some(qualifier) = qualifier {
            let qualifier = Intern::from_ref(qualifier);
            if !decl
                .provided_traits
                .iter()
                .any(|p| p.trait_name == qualifier)
            {
                return Vec::new();
            }
            contracts.clear();
            let provided = decl
                .provided_traits
                .iter()
                .find(|p| p.trait_name == qualifier)
                .unwrap();
            self.collect(
                qualifier,
                &provided.args,
                &mut HashSet::new(),
                &mut contracts,
            );
        }
        contracts.retain(|contract| contract.method.name.as_str() == name);
        contracts
    }

    pub fn effective<'b>(&self, bind: &'b Bind) -> Cow<'b, Bind> {
        let contracts = self.contracts(bind);
        let Some(contract) = contracts.first() else {
            return Cow::Borrowed(bind);
        };
        let mut effective = bind.clone();
        if effective.return_tag.is_none()
            && effective.return_type_name.is_none()
            && effective.type_annotation.is_none()
            && effective.anonymous_result_alternatives.is_empty()
        {
            effective.return_tag = contract.method.return_ty.clone();
            effective.anonymous_result_alternatives =
                contract.method.anonymous_result_alternatives.clone();
        }
        for contract in contracts {
            let names: HashMap<_, _> = contract
                .method
                .params
                .keys()
                .copied()
                .zip(
                    effective
                        .params
                        .iter()
                        .flat_map(|params| params.keys().copied()),
                )
                .collect();
            for proof in &contract.method.payload_postconditions {
                let mut proof = proof.clone();
                rename_payload_inputs(&mut proof, &names);
                if !effective.payload_postconditions.contains(&proof) {
                    effective.payload_postconditions.push(proof);
                }
            }
            effective.attributes.inline_always |= contract.method.attributes.inline_always;
            if effective.attributes.infix_symbol.is_none() {
                effective.attributes.infix_symbol = contract.method.attributes.infix_symbol;
                effective.attributes.infix_symbol_span = Some(bind.name_span);
            }
        }
        Cow::Owned(effective)
    }

    pub fn public_signature<'b>(&self, bind: &'b Bind) -> Cow<'b, Bind> {
        let mut signature = self.effective(bind);
        let contracts = self.contracts(bind);
        let Some(contract) = contracts.first() else {
            return signature;
        };
        if let Some(params) = &mut signature.to_mut().params {
            for (parameter, required) in params.values_mut().zip(contract.method.params.values()) {
                if matches!(&required.kind, ParameterKind::Tagged(surface) if matches!(surface.value, Expr::IntegerDomain(_)))
                {
                    continue;
                }
                if let ParameterKind::Tagged(surface) = &mut parameter.kind
                    && let Expr::IntegerDomain(inner) = &surface.value
                {
                    surface.value = inner.value.clone();
                }
            }
        }
        signature
    }

    pub fn expand_defaults(&self) -> Cow<'a, FileAst> {
        let mut expanded = Cow::Borrowed(self.file);
        for (name, bind) in &self.file.defs {
            if self.receiver(bind).is_some_and(|decl| matches!(&decl.value, DeclareValue::Has(members) if members.iter().all(|m| matches!(m, HasMember::Function(_)))) && has_domain_projection(&bind.value, decl.name))
            {
                expanded.to_mut().defs.shift_remove(name);
            }
        }
        for decl in self.file.tags.values() {
            let contracts = self.required(decl);
            for contract in &contracts {
                if contract.method.body.is_none() {
                    continue;
                }
                let explicit = self.file.defs.values().any(|bind| {
                    self.receiver(bind)
                        .is_some_and(|receiver| receiver.name == decl.name)
                        && self.contracts(bind).iter().any(|c| {
                            c.method.name == contract.method.name && c.owner == contract.owner
                        })
                });
                if explicit {
                    continue;
                }
                let competing = contracts
                    .iter()
                    .filter(|c| c.method.name == contract.method.name && c.method.body.is_some())
                    .count();
                if competing != 1 {
                    continue;
                }
                let receiver = decl.receiver_surface();
                let mut bind = contract.method.implementation(receiver).unwrap();
                bind.name = contract.method.name;
                bind.name_span = decl.name_span;
                let prefix = self
                    .file
                    .defs
                    .iter()
                    .find_map(|(key, existing)| {
                        self.receiver(existing)
                            .filter(|r| r.name == decl.name)
                            .and_then(|_| {
                                key.0
                                    .as_str()
                                    .rfind(&format!("{}.", decl.name))
                                    .map(|offset| key.0.as_str()[..offset].to_string())
                            })
                    })
                    .unwrap_or_else(|| {
                        self.file
                            .semantic_origin
                            .as_ref()
                            .map(|origin| {
                                let module = origin
                                    .module
                                    .iter()
                                    .map(|p| p.as_str())
                                    .collect::<Vec<_>>()
                                    .join(".");
                                if module.is_empty() {
                                    module
                                } else {
                                    format!("{module}.")
                                }
                            })
                            .unwrap_or_default()
                    });
                let key = Intern::new(format!("{prefix}{}.{}", decl.name, bind.name));
                expanded.to_mut().method_binds.push(bind.clone());
                expanded.to_mut().defs.insert(key, bind);
            }
        }
        expanded
    }

    fn check_qualifier(&self, decl: &Declare, qualifier: &str, name: &str) -> Option<Diagnostic> {
        if qualifier == decl.name.as_str() {
            return None;
        }
        let mut methods = Vec::new();
        self.collect(
            Intern::from_ref(qualifier),
            &[],
            &mut HashSet::new(),
            &mut methods,
        );
        let (code, message) = if !decl
            .provided_traits
            .iter()
            .any(|p| p.trait_name.as_str() == qualifier)
        {
            (
                // type-uninherited-trait-qualifier
                DiagnosticCode::Type(TypeCode::Trait(TraitCode::UninheritedQualifier)),
                format!("`{}` does not inherit trait `{qualifier}`", decl.name),
            )
        } else if self.declaration(Intern::from_ref(qualifier)).is_none() {
            (
                DiagnosticCode::Type(TypeCode::Trait(TraitCode::Unknown)),
                format!("unknown trait `{qualifier}`"),
            )
        } else if !methods.iter().any(|c| c.method.name.as_str() == name) {
            (
                DiagnosticCode::Type(TypeCode::Trait(TraitCode::UnknownMethod)),
                format!("trait `{qualifier}` has no method `{name}`"),
            )
        } else {
            return None;
        };
        Some(
            Diagnostic::new(code, message)
                .with_arg("trait_name", qualifier.to_string())
                .with_arg("method_name", name.to_string()),
        )
    }

    pub fn check_implementations(&self, mut report: impl FnMut(ast::SpanId, Diagnostic)) {
        for bind in self.file.defs.values() {
            let Some(decl) = self.receiver(bind) else {
                continue;
            };
            let Some((qualifier, name)) = self.member_name(bind).split_once('.') else {
                continue;
            };
            if let Some(diagnostic) = self.check_qualifier(decl, qualifier, name) {
                report(bind.name_span, diagnostic);
            }
        }
        for decl in self.file.tags.values() {
            for provided in &decl.provided_traits {
                if let Some(trait_decl) = self.declaration(provided.trait_name) {
                    let expected = trait_decl.params.as_ref().map_or(0, |p| p.len());
                    if provided.args.len() != expected {
                        report(
                            provided.trait_name_span,
                            // type-trait-argument-count
                            Diagnostic::new(
                                TraitCode::ArgumentCount,
                                format!(
                                    "trait `{}` requires {expected} type arguments, found {}",
                                    provided.trait_name,
                                    provided.args.len()
                                ),
                            ),
                        );
                    }
                }
            }
            let contracts = self.required(decl);
            for contract in &contracts {
                if contract.method.body.is_some()
                    && contracts
                        .iter()
                        .filter(|c| {
                            c.method.name == contract.method.name && c.method.body.is_some()
                        })
                        .count()
                        > 1
                {
                    let explicit = self.file.defs.values().any(|b| {
                        self.receiver(b).is_some_and(|r| r.name == decl.name)
                            && self.contracts(b).iter().any(|c| {
                                c.owner == contract.owner && c.method.name == contract.method.name
                            })
                    });
                    if !explicit {
                        report(
                            decl.name_span,
                            // type-ambiguous-trait-default
                            Diagnostic::new(
                                TraitCode::AmbiguousDefault,
                                format!(
                                    "conflicting defaults for `{}` require an explicit implementation",
                                    contract.method.name
                                ),
                            ),
                        );
                    }
                }
            }
            if let DeclareValue::Has(members) = &decl.value {
                for member in members {
                    if let HasMember::Function(method) = member
                        && method.body.is_none()
                        && let Some(qualifier) = &method.qualifier
                        && let Some(diagnostic) = self.check_qualifier(
                            decl,
                            qualifier.name.as_str(),
                            method.name.as_str(),
                        )
                    {
                        report(method.name_span, diagnostic);
                    }
                }
            }
            if self.is_interface(decl) {
                continue;
            }
            for contract in self.unimplemented(decl) {
                report(
                    decl.name_span,
                    // type-missing-trait-method
                    Diagnostic::new(
                        TraitCode::MissingMethod,
                        format!(
                            "`{}` must implement `{}.{}`",
                            decl.name, contract.owner, contract.method.name
                        ),
                    )
                    .with_arg("trait_name", contract.owner.as_str().to_string())
                    .with_arg("method_name", contract.method.name.as_str().to_string()),
                );
            }
        }
    }
}

fn specialize_method(method: &HasFunction, subst: &HashMap<Intern<String>, Expr>) -> HasFunction {
    let mut method = method.clone();
    let mut folder = ParameterSubstitution::new(subst);
    for p in method.params.values_mut() {
        match &mut p.kind {
            ParameterKind::Tagged(surface)
            | ParameterKind::ValueParam { ty: surface }
            | ParameterKind::Inferred { ty: surface } => {
                let _ = folder.visit_expr(&mut surface.value);
            }
            _ => {}
        }
    }
    for proof in &mut method.payload_postconditions {
        let _ = folder.visit_expr(&mut proof.ty.value);
    }
    if let Some(surface) = &mut method.return_ty {
        let _ = folder.visit_expr(&mut surface.value);
    }
    if let Some(ast::HasMemberBody::Overrideable(value) | ast::HasMemberBody::Final(value)) =
        &mut method.body
    {
        let _ = folder.visit_bind_value(value);
    }
    method
}

fn has_domain_projection(value: &ast::BindValue, receiver: Intern<String>) -> bool {
    use ast::folder::{Visitor, walk_expr};
    use std::ops::ControlFlow;
    struct Projection(Intern<String>);
    impl Visitor for Projection {
        fn visit_expr(&mut self, expr: &Expr) -> ControlFlow<()> {
            if matches!(expr, Expr::IntegerDomain(inner) if matches!(&inner.value, Expr::AnonymousTag(name) if *name == self.0 || name.as_str() == "Self"))
            {
                return ControlFlow::Break(());
            }
            if let Expr::Bind(bind) = expr
                && let Some(surface) = &bind.return_tag
            {
                self.visit_expr(&surface.value)?;
            }
            walk_expr(self, expr)
        }
    }
    Projection(receiver).visit_bind_value(value).is_break()
}

fn rename_payload_inputs(
    proof: &mut ast::PayloadPostcondition,
    names: &HashMap<Intern<String>, Intern<String>>,
) {
    struct Renamer<'a> {
        payload: Intern<String>,
        names: &'a HashMap<Intern<String>, Intern<String>>,
    }

    impl ProofVisitorMut for Renamer<'_> {
        fn visit_term(&mut self, term: &mut ast::ProofTerm) -> ControlFlow<()> {
            if let ast::ProofTerm::Name(name) = term {
                let (root, suffix) = name
                    .as_str()
                    .split_once('.')
                    .map_or((name.as_str(), ""), |(root, suffix)| (root, suffix));
                let root = Intern::from_ref(root);
                if root != self.payload
                    && let Some(replacement) = self.names.get(&root)
                {
                    *name = if suffix.is_empty() {
                        *replacement
                    } else {
                        Intern::new(format!("{replacement}.{suffix}"))
                    };
                }
            }
            walk_term(self, term)
        }
    }

    let _ = Renamer {
        payload: proof.name,
        names,
    }
    .visit_proposition(&mut proof.proposition);
}
