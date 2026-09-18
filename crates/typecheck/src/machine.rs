use std::collections::{BTreeSet, HashMap, HashSet};

use ast::parameter::ParameterKind;
use ast::{AsmOperandBinding, BinOp, Bind, BindOperator, BindValue, Expr, FileAst, Typed};
use diagnostic::Diagnostic;
use flask::CompileTarget;

pub(crate) mod backend;
use internment::Intern;

use crate::assembly::{AssemblyInput, AssemblyMemory, AssemblySpec};
use crate::{DefId, TypedFileAst};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Immutable,
    Mutable,
}

#[derive(Clone)]
struct RegisterBinding {
    register: String,
    access: Access,
}

type Scope = HashMap<Intern<String>, RegisterBinding>;

struct Signature {
    word: Intern<String>,
    parameters: Vec<(Intern<String>, Access)>,
    returns_value: bool,
}

struct ReferenceInput {
    parameter: String,
    pointer: String,
}

struct Instruction {
    operation: &'static str,
    destination: String,
    left: String,
    right: String,
    boundary: bool,
}

impl Instruction {
    fn transfer(operation: &'static str, destination: &str, source: &str) -> Self {
        Self {
            operation,
            destination: destination.into(),
            left: source.into(),
            right: source.into(),
            boundary: false,
        }
    }

    fn text(value: String, span: span::SpanId) -> Typed<Expr> {
        Typed::resolved(
            Expr::Lit(ast::Literal::String(value.clone())),
            ast::ty::Ty::Literal(ast::ConstValue::Text(value)),
            span,
        )
    }

    fn expression(&self, span: span::SpanId) -> Typed<Expr> {
        let mut fields: Vec<_> = [
            ("operation", self.operation),
            ("destination", &self.destination),
            ("left", &self.left),
            ("right", &self.right),
        ]
        .into_iter()
        .map(|(name, value)| (Intern::from_ref(name), Self::text(value.into(), span)))
        .collect();
        fields.push((
            Intern::from_ref("boundary"),
            Typed::infer(
                Expr::Lit(ast::Literal::Int(i64::from(self.boundary).into())),
                span,
            ),
        ));
        Typed::infer(Expr::RecordLit(fields), span)
    }
}

pub(crate) struct MachineDefinitions(HashMap<Intern<String>, Bind>);

impl MachineDefinitions {
    pub(crate) fn new<'a>(files: impl IntoIterator<Item = &'a FileAst>) -> Self {
        Self(
            files
                .into_iter()
                .flat_map(|file| file.defs.iter())
                .map(|(name, bind)| (*name, bind.clone()))
                .collect(),
        )
    }

    pub(crate) fn lower(
        &self,
        file: &mut FileAst,
        target: &CompileTarget,
    ) -> Vec<(DefId, Diagnostic)> {
        let mut flaws = Vec::new();
        for (name, bind) in &mut file.defs {
            if !bind.attributes.machine {
                continue;
            }
            match Region::lower(bind, self, target) {
                Ok(expr) => bind.value = BindValue::Expr(Box::new(expr)),
                Err(message) => {
                    flaws.push((
                        DefId(*name),
                        Diagnostic::new("type-machine", message)
                            .at_span_id(bind.name_span, &file.span_table),
                    ));
                    bind.value = BindValue::Expr(Box::new(Typed::infer(
                        Expr::Invalid(bind.name_span),
                        bind.name_span,
                    )));
                }
            }
        }
        flaws
    }

    pub(crate) fn attach(typed: &mut TypedFileAst, flaws: Vec<(DefId, Diagnostic)>) {
        for (id, flaw) in flaws {
            if let Some(bind) = typed.defs.get_mut(&id) {
                bind.flaws.push(flaw);
            }
        }
    }

    fn validate_leaf_operands(
        &self,
        bind: &Bind,
        operands: &[AsmOperandBinding],
    ) -> Result<(), String> {
        let mut pending: Vec<_> = operands
            .iter()
            .map(|operand| &operand.value.value)
            .collect();
        while let Some(expr) = pending.pop() {
            if let Expr::FnCall(call) = expr {
                let name = call.mangled_name();
                let parameter = call.args.is_none()
                    && bind
                        .params
                        .as_ref()
                        .is_some_and(|params| params.contains_key(&name));
                let permitted = self.0.get(&name).is_some_and(|callee| {
                    callee.attributes.machine || (call.args.is_none() && callee.is_constant())
                });
                if !parameter && !permitted {
                    return Err(format!(
                        "#machine cannot call `{name}` in an assembly operand: the callee must also have #machine"
                    ));
                }
            }
            if let Expr::Asm { operands, .. } = expr {
                pending.extend(operands.iter().map(|operand| &operand.value.value));
            } else {
                let _ = ast::folder::walk_expr_children(expr, &mut |_, child| {
                    pending.push(child);
                    std::ops::ControlFlow::Continue(())
                });
            }
        }
        Ok(())
    }

    pub(crate) fn validate_signatures(typed: &mut TypedFileAst, file: &FileAst) {
        let signed =
            ast::integer::IntegerDomain::bounded(i64::MIN.into(), i64::MAX.into()).unwrap();
        let unsigned = ast::integer::IntegerDomain::bounded(0.into(), u64::MAX.into()).unwrap();
        for bind in typed
            .defs
            .values_mut()
            .filter(|bind| bind.attributes.machine)
        {
            if file.defs.get(&bind.name).is_some_and(|source| {
                matches!(&source.value, BindValue::Expr(expr)
                    if matches!(&expr.value, Expr::Asm { specification, .. }
                        if matches!(&specification.value, Expr::RecordLit(fields)
                            if fields.iter().any(|(name, _)| name.as_str() == "machine_leaf"))))
            }) {
                continue;
            }
            if !matches!(bind.body, crate::BindBody::Expr(expr)
                if matches!(typed.exprs.kind_of(expr), Some(crate::TypedExprKind::Asm { .. })))
            {
                if !bind
                    .flaws
                    .iter()
                    .any(|flaw| flaw.code.slug() == "type-machine")
                {
                    bind.flaws.push(
                        Diagnostic::new(
                            "type-machine",
                            "the initial #machine context supports top-level callables only",
                        )
                        .at_span_id(bind.name_span, &typed.span_table),
                    );
                }
                continue;
            }
            let ty = bind
                .params
                .first()
                .map(|(_, ty)| ty.without_reference_wrappers())
                .unwrap_or(&bind.return_type);
            let valid = matches!(
                crate::representation::Repr::derive(ty, Some(&typed.type_registry)),
                Ok(crate::representation::Repr::Bits { width: 64 })
            ) && typed
                .type_registry
                .integer_validity_for_type(ty)
                .is_some_and(|validity| {
                    validity.domain() == &signed || validity.domain() == &unsigned
                })
                && bind
                    .params
                    .iter()
                    .all(|(_, parameter)| parameter.without_reference_wrappers() == ty)
                && (bind.return_type == crate::ty::Ty::Unit || &bind.return_type == ty);
            if !valid
                && !bind
                    .flaws
                    .iter()
                    .any(|flaw| flaw.code.slug() == "type-machine")
            {
                bind.flaws.push(Diagnostic::new("type-machine",
                    "#machine currently requires a full signed or unsigned 64-bit integer domain for parameters and result")
                    .at_span_id(bind.name_span, &typed.span_table));
            }
        }
    }
}

struct Region<'a> {
    definitions: &'a MachineDefinitions,
    word: Intern<String>,
    inputs: Vec<AssemblyInput>,
    initialized: HashSet<String>,
    frozen: HashSet<String>,
    references: HashMap<String, ReferenceInput>,
    dirty: HashSet<String>,
    memory: AssemblyMemory,
    length_parameter: String,
    occupied: HashSet<String>,
    written: BTreeSet<String>,
    instructions: Vec<Instruction>,
    registers: BTreeSet<String>,
    backends: Vec<Typed<Expr>>,
    calls: Vec<Intern<String>>,
    expansions: usize,
}

impl<'a> Region<'a> {
    fn lower(
        bind: &Bind,
        definitions: &'a MachineDefinitions,
        target: &'a CompileTarget,
    ) -> Result<Typed<Expr>, String> {
        let CompileTarget::Concrete(target) = target else {
            return Err("#machine requires a concrete compile target".into());
        };
        let backend = Self::backend(bind)?;
        if let Some(leaf) = Self::assembly_leaf(bind) {
            let Expr::Asm {
                specification,
                operands,
            } = &leaf.value
            else {
                unreachable!()
            };
            definitions.validate_leaf_operands(bind, operands)?;
            let specification = match &specification.value {
                Expr::Lit(ast::Literal::String(source)) => {
                    Instruction::text(source.clone(), specification.span_id)
                }
                _ => (**specification).clone(),
            };
            return Ok(Typed::infer(
                Expr::Asm {
                    specification: Box::new(Typed::infer(
                        Expr::RecordLit(vec![
                            (Intern::from_ref("machine_leaf"), backend.clone()),
                            (Intern::from_ref("specification"), specification),
                        ]),
                        leaf.span_id,
                    )),
                    operands: operands.clone(),
                },
                leaf.span_id,
            ));
        }
        let registers = ["$input0", "$input1"];
        let signature = Self::signature(bind)?;
        let word = signature.word;
        let params = bind
            .params
            .as_ref()
            .ok_or("#machine requires an explicit parameter list")?;
        if params.len() > registers.len() {
            return Err("the initial #machine boundary supports at most two parameters".into());
        }
        let inputs: Vec<_> = params
            .keys()
            .zip(&registers)
            .map(|(name, register)| AssemblyInput {
                parameter: name.as_str().into(),
                register: (*register).into(),
                tie: false,
            })
            .collect();
        let mut inputs = inputs;
        let mut references = HashMap::new();
        let mut scope = Scope::new();
        let mut initialized = HashSet::new();
        let mut occupied: HashSet<_> = inputs.iter().map(|input| input.register.clone()).collect();
        let scratch = ["$reference0", "$reference1"];
        for (index, ((name, access), input)) in signature.parameters.iter().zip(&inputs).enumerate()
        {
            let register = if *access == Access::Mutable {
                let register = scratch[index].to_string();
                references.insert(
                    register.clone(),
                    ReferenceInput {
                        parameter: input.parameter.clone(),
                        pointer: input.register.clone(),
                    },
                );
                occupied.insert(register.clone());
                register
            } else {
                initialized.insert(input.register.clone());
                input.register.clone()
            };
            scope.insert(
                *name,
                RegisterBinding {
                    register,
                    access: *access,
                },
            );
        }
        let mut length_parameter = "__machine_bytes".to_string();
        while params.contains_key(&Intern::from_ref(&length_parameter)) {
            length_parameter.push('_');
        }
        if !references.is_empty() {
            let register = "$length";
            occupied.insert(register.into());
            inputs.push(AssemblyInput {
                parameter: length_parameter.clone(),
                register: register.into(),
                tie: false,
            });
        }
        let mut region = Self {
            definitions,
            word,
            initialized,
            occupied,
            inputs,
            references,
            frozen: HashSet::new(),
            dirty: HashSet::new(),
            memory: AssemblyMemory::default(),
            length_parameter,
            written: BTreeSet::new(),
            instructions: Vec::new(),
            calls: vec![bind.name],
            expansions: 0,
            registers: BTreeSet::new(),
            backends: Vec::new(),
        };
        let result = region.body(bind, &mut scope, None)?;
        let mut references: Vec<_> = region.references.iter().collect();
        references.sort_by_key(|(_, input)| &input.parameter);
        for (register, input) in references {
            if region.dirty.contains(register) {
                region
                    .instructions
                    .push(Instruction::transfer("store", &input.pointer, register));
                region
                    .memory
                    .writes
                    .push((input.parameter.clone(), region.length_parameter.clone()));
            }
        }
        for input in &mut region.inputs {
            input.tie = result.as_ref() == Some(&input.register);
        }
        let clobbers: Vec<_> = region
            .written
            .into_iter()
            .filter(|register| result.as_ref() != Some(register))
            .collect();
        let spec = AssemblySpec {
            target: target.raw.clone(),
            dialect: "att".into(),
            inputs: region.inputs,
            result,
            outputs: Vec::new(),
            clobbers,
            memory: region.memory,
            side_effects: true,
            template: String::new(),
        };
        let mut operands: Vec<_> = signature
            .parameters
            .iter()
            .map(|(name, access)| {
                let value = Typed::infer(
                    Expr::FnCall(ast::FnCall {
                        path: ast::Spanned::new(
                            ast::ModPath::new(*name, Vec::new()),
                            bind.name_span,
                        ),
                        args: None,
                    }),
                    bind.name_span,
                );
                let value = if *access == Access::Mutable {
                    Typed::infer(
                        Expr::Ref {
                            inner: Box::new(value),
                            mutable: true,
                            group: None,
                        },
                        bind.name_span,
                    )
                } else {
                    value
                };
                AsmOperandBinding { name: *name, value }
            })
            .collect();
        if !region.references.is_empty() {
            operands.push(AsmOperandBinding {
                name: Intern::from_ref(&region.length_parameter),
                value: Typed::infer(
                    Expr::Cast {
                        expr: Box::new(Typed::infer(
                            Expr::Lit(ast::Literal::Int(8.into())),
                            bind.name_span,
                        )),
                        ty: ast::TypeReference::Nominal(word),
                    },
                    bind.name_span,
                ),
            });
        }
        let span = bind.name_span;
        let fields = vec![
            (Intern::from_ref("machine_backend"), backend.clone()),
            (
                Intern::from_ref("backends"),
                Typed::infer(Expr::List(region.backends), span),
            ),
            (
                Intern::from_ref("contract"),
                Instruction::text(spec.canonical_form(), span),
            ),
            (
                Intern::from_ref("instructions"),
                Typed::infer(
                    Expr::List(
                        region
                            .instructions
                            .iter()
                            .map(|instruction| instruction.expression(span))
                            .collect(),
                    ),
                    span,
                ),
            ),
            (
                Intern::from_ref("registers"),
                Typed::infer(
                    Expr::List(
                        region
                            .registers
                            .into_iter()
                            .map(|register| Instruction::text(register, span))
                            .collect(),
                    ),
                    span,
                ),
            ),
        ];
        Ok(Typed::infer(
            Expr::Asm {
                specification: Box::new(Typed::infer(Expr::RecordLit(fields), span)),
                operands,
            },
            span,
        ))
    }

    fn assembly_leaf(bind: &Bind) -> Option<&Typed<Expr>> {
        let expr = match &bind.value {
            BindValue::Expr(expr) => expr.as_ref(),
            BindValue::Body { exprs, ret } => match (exprs.as_slice(), ret.values.as_slice()) {
                ([], [expr]) | ([expr], []) => expr,
                _ => return None,
            },
            _ => return None,
        };
        matches!(expr.value, Expr::Asm { .. }).then_some(expr)
    }

    fn backend(bind: &Bind) -> Result<&Typed<Expr>, String> {
        let backend = bind
            .attributes
            .raw_attributes
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|attribute| match attribute {
                ast::AttributeItem::Call { name, args, .. } if name.as_str() == "machine" => {
                    Some(args)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let [arguments] = backend.as_slice() else {
            return Err("#machine requires one backend value: #machine(architecture)".into());
        };
        let [backend] = arguments.as_slice() else {
            return Err("#machine requires exactly one backend value".into());
        };
        Ok(backend)
    }

    fn signature(bind: &Bind) -> Result<Signature, String> {
        if bind.receiver_type.is_some()
            || !bind.named_results.is_empty()
            || bind.is_constant()
            || bind.return_refinement.is_some()
            || !bind.param_refinements.is_empty()
            || !bind.group_params.is_empty()
            || !bind.anonymous_result_alternatives.is_empty()
        {
            return Err(
                "#machine requires an ordinary callable with a machine-word or unit result".into(),
            );
        }
        let result = bind
            .return_tag
            .as_ref()
            .map(|ty| Self::type_name(&ty.value))
            .unwrap_or(bind.return_type_name);
        if bind.return_tag.is_some() && result.is_none() {
            return Err("unsupported #machine return type".into());
        }
        let mut word = result;
        let params = bind
            .params
            .as_ref()
            .ok_or("#machine requires an explicit parameter list")?;
        let mut parameters = Vec::new();
        for (name, parameter) in params {
            let ParameterKind::Tagged(ty) = &parameter.kind else {
                return Err("#machine parameters require explicit machine-word types".into());
            };
            let (ty, access) = match &ty.value {
                Expr::Ref {
                    inner,
                    mutable: true,
                    group: None,
                } => (&inner.value, Access::Mutable),
                expr => (expr, Access::Immutable),
            };
            let ty = Self::type_name(ty)
                .ok_or("#machine parameters must be machine words or mutable machine words")?;
            if *word.get_or_insert(ty) != ty || !parameter.constraints.is_empty() {
                return Err(
                    "#machine parameters and result must use the same machine-word type".into(),
                );
            }
            parameters.push((*name, access));
        }
        Ok(Signature {
            word: word.ok_or("#machine requires a machine-word parameter or result")?,
            parameters,
            returns_value: result.is_some(),
        })
    }

    fn type_name(expr: &Expr) -> Option<Intern<String>> {
        match expr {
            Expr::AnonymousTag(name) => Some(*name),
            Expr::FnCall(call) if call.args.is_none() => Some(call.mangled_name()),
            Expr::TagCall(call) if call.args.is_empty() => Some(call.name),
            _ => None,
        }
    }

    fn body(
        &mut self,
        bind: &Bind,
        scope: &mut Scope,
        destination: Option<&str>,
    ) -> Result<Option<String>, String> {
        let signature = Self::signature(bind)?;
        match &bind.value {
            BindValue::Expr(expr) if signature.returns_value => {
                self.expression(&expr.value, scope, destination).map(Some)
            }
            BindValue::Body { exprs, ret } => {
                for expr in exprs {
                    match &expr.value {
                        Expr::Bind(local) => self.bind(local, scope)?,
                        Expr::FnCall(call) => {
                            if self.call(call, scope, None)?.is_some() {
                                return Err(
                                    "bind or return the result of a value-producing machine helper"
                                        .into(),
                                );
                            }
                        }
                        _ => return Err("unsupported statement in #machine".into()),
                    }
                }
                match ret.values.as_slice() {
                    [] if !signature.returns_value => Ok(None),
                    [value] if signature.returns_value => {
                        self.expression(&value.value, scope, destination).map(Some)
                    }
                    _ => Err("#machine return does not match its declared result".into()),
                }
            }
            _ => Err("#machine requires a Gin expression or body".into()),
        }
    }

    fn bind(&mut self, local: &Bind, scope: &mut Scope) -> Result<(), String> {
        let BindValue::Expr(value) = &local.value else {
            return Err("#machine register bindings require an expression".into());
        };
        if local.params.is_some()
            || local.return_type_name.is_some()
            || local.return_tag.is_some()
            || local.type_annotation.is_some()
            || local.attributes.raw_attributes.is_some()
        {
            return Err(
                "the initial #machine context supports unannotated register bindings".into(),
            );
        }
        match local.operator {
            BindOperator::FreshMutable => {
                let Expr::FnCall(call) = &value.value else {
                    return Err(
                        "introduce a physical register with a binding such as `left: rax`".into(),
                    );
                };
                let register = call.mangled_name().as_str().to_string();
                if call.args.is_some() || register.starts_with('$') {
                    return Err(format!(
                        "`{register}` is not an available scratch register for this #machine target"
                    ));
                }
                if scope.contains_key(&local.name) || !self.occupied.insert(register.clone()) {
                    return Err(format!(
                        "register binding `{}` overlaps an existing binding or input",
                        local.name
                    ));
                }
                self.registers.insert(register.clone());
                self.initialized.remove(&register);
                scope.insert(
                    local.name,
                    RegisterBinding {
                        register,
                        access: Access::Mutable,
                    },
                );
            }
            BindOperator::FreshImmutable => {
                if scope.contains_key(&local.name) {
                    return Err(format!(
                        "register binding `{}` overlaps an existing binding",
                        local.name
                    ));
                }
                let register = self.read(&value.value, scope)?;
                self.frozen.insert(register.clone());
                scope.insert(
                    local.name,
                    RegisterBinding {
                        register,
                        access: Access::Immutable,
                    },
                );
            }
            BindOperator::Rebind => {
                let binding = scope
                    .get(&local.name)
                    .ok_or_else(|| format!("unknown register binding `{}`", local.name))?;
                self.require_mutable(binding)?;
                self.expression(&value.value, scope, Some(&binding.register))?;
            }
            _ => return Err("use `:`, `:=`, or `::` for machine register bindings".into()),
        }
        Ok(())
    }

    fn require_mutable(&self, binding: &RegisterBinding) -> Result<(), String> {
        if binding.access != Access::Mutable || self.frozen.contains(&binding.register) {
            return Err(format!(
                "cannot update immutable register `{}`",
                binding.register
            ));
        }
        Ok(())
    }

    fn read(&mut self, expr: &Expr, scope: &Scope) -> Result<String, String> {
        if let Expr::FnCall(call) = expr
            && call.args.is_none()
            && let Some(binding) = scope.get(&call.mangled_name())
        {
            let register = &binding.register;
            if !self.initialized.contains(register) {
                if let Some(input) = self.references.get(register) {
                    self.instructions
                        .push(Instruction::transfer("load", register, &input.pointer));
                    self.memory
                        .reads
                        .push((input.parameter.clone(), self.length_parameter.clone()));
                    self.written.insert(register.clone());
                    self.initialized.insert(register.clone());
                } else {
                    return Err(format!(
                        "register `{}` is read before initialization",
                        call.mangled_name()
                    ));
                }
            }
            return Ok(register.clone());
        }
        Err("machine operands must be initialized register bindings or input values".into())
    }

    fn call(
        &mut self,
        call: &ast::FnCall,
        scope: &Scope,
        destination: Option<&str>,
    ) -> Result<Option<String>, String> {
        self.expansions += 1;
        if self.expansions > 4096 {
            return Err("#machine region exceeds 4096 helper expansions".into());
        }
        let name = call.mangled_name();
        let bind = self
            .definitions
            .0
            .get(&name)
            .filter(|bind| bind.attributes.machine)
            .ok_or_else(|| {
                format!("#machine cannot call `{name}`: the callee must also have #machine")
            })?
            .clone();
        if Self::assembly_leaf(&bind).is_some() {
            return Err(format!(
                "cannot expand assembly machine leaf `{name}` inside a register body; call it from ordinary Gin"
            ));
        }
        if self.calls.contains(&name) {
            return Err(format!(
                "recursive #machine helper expansion through `{name}`"
            ));
        }
        if self.calls.len() >= 64 {
            return Err("#machine helper expansion exceeds 64 nested calls".into());
        }
        self.backends.push(Self::backend(&bind)?.clone());
        let signature = Self::signature(&bind)?;
        if signature.word != self.word {
            return Err("machine helpers must use the caller's machine-word type".into());
        }
        let args = call.args.as_deref().unwrap_or_default();
        if signature.parameters.len() != args.len() {
            return Err(format!(
                "machine helper `{name}` expects {} arguments, got {}",
                signature.parameters.len(),
                args.len()
            ));
        }
        let mut child = Scope::new();
        let mut aliases = HashMap::new();
        for ((parameter, access), arg) in signature.parameters.iter().zip(args) {
            let register = if *access == Access::Mutable {
                let Expr::FnCall(argument) = &arg.value else {
                    return Err(
                        "a mutating machine parameter requires a mutable register binding".into(),
                    );
                };
                let binding = scope
                    .get(&argument.mangled_name())
                    .filter(|_| argument.args.is_none())
                    .ok_or("a mutating machine parameter requires a mutable register binding")?;
                self.require_mutable(binding)?;
                binding.register.clone()
            } else {
                self.read(&arg.value, scope)?
            };
            if let Some(previous) = aliases.insert(register.clone(), *access)
                && (previous == Access::Mutable || *access == Access::Mutable)
            {
                return Err(
                    "overlapping machine arguments cannot include a mutable register parameter"
                        .into(),
                );
            }
            child.insert(
                *parameter,
                RegisterBinding {
                    register,
                    access: *access,
                },
            );
        }
        self.calls.push(name);
        let frozen = self.frozen.clone();
        let occupied = self.occupied.clone();
        let result = self.body(&bind, &mut child, destination);
        self.frozen = frozen;
        self.occupied = occupied;
        self.calls.pop();
        result
    }

    fn expression(
        &mut self,
        expr: &Expr,
        scope: &Scope,
        destination: Option<&str>,
    ) -> Result<String, String> {
        if self.instructions.len() >= 4096 {
            return Err("#machine region exceeds 4096 instructions".into());
        }
        if let Expr::FnCall(call) = expr {
            if call.args.is_none() && scope.contains_key(&call.mangled_name()) {
                let source = self.read(expr, scope)?;
                if let Some(destination) = destination {
                    self.copy(destination, &source)?;
                    return Ok(destination.into());
                }
                return Ok(source);
            }
            return self
                .call(call, scope, destination)?
                .ok_or_else(|| "a unit machine helper cannot be used as a value".into());
        }
        if let Expr::Binary(binary) = expr {
            if binary.op != BinOp::Add {
                return Err(
                    "the initial #machine context supports register copies and addition".into(),
                );
            }
            let left = self.read(&binary.lhs.value, scope)?;
            let right = self.read(&binary.rhs.value, scope)?;
            let result = match destination {
                Some(destination) => destination.to_string(),
                None => {
                    let register = "$result";
                    if self.occupied.contains(register) {
                        return Err("return the result through an explicit register binding".into());
                    }
                    register.into()
                }
            };
            self.instructions.push(Instruction {
                operation: "add",
                destination: result.clone(),
                left,
                right,
                boundary: destination.is_none(),
            });
            if self.frozen.contains(&result) {
                return Err(format!("cannot update immutable register `{result}`"));
            }
            self.dirty.insert(result.clone());
            self.initialized.insert(result.clone());
            self.written.insert(result.clone());
            return Ok(result);
        }
        Err(
            "unsupported expression in #machine; use register copies, addition, or machine helpers"
                .into(),
        )
    }

    fn copy(&mut self, destination: &str, source: &str) -> Result<(), String> {
        if destination == source {
            return Ok(());
        }
        if self.frozen.contains(destination) {
            return Err(format!("cannot update immutable register `{destination}`"));
        }
        self.dirty.insert(destination.into());
        self.instructions
            .push(Instruction::transfer("copy", destination, source));
        self.written.insert(destination.into());
        self.initialized.insert(destination.into());
        Ok(())
    }
}
