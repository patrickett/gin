use diagnostic::code::EmissionCode;
use internment::Intern;
use std::collections::HashSet;

use crate::ty::Ty;
use crate::typed::TypedFileAst;
use ast::ConstValue;
use diagnostic::Diagnostic;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblySpec {
    pub target: String,
    pub dialect: String,
    pub inputs: Vec<AssemblyInput>,
    pub result: Option<String>,
    pub outputs: Vec<AssemblyOutput>,
    pub clobbers: Vec<String>,
    pub memory: AssemblyMemory,
    pub side_effects: bool,
    pub template: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssemblyState {
    Pending,
    Checked(Box<CheckedAssembly>),
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedAssembly {
    pub target: String,
    pub dialect: String,
    pub template: String,
    pub inputs: Vec<AssemblyInput>,
    pub operands: Vec<(Intern<String>, crate::typed::ExprId)>,
    pub record: crate::typed::ExprId,
    pub construction: Vec<crate::typed::ExprId>,
    pub result: Option<String>,
    pub outputs: Vec<AssemblyOutput>,
    pub clobbers: Vec<String>,
    pub memory: AssemblyMemory,
    pub side_effects: bool,
    pub tied_output_indices: Vec<Option<usize>>,
}

impl CheckedAssembly {
    fn from_spec(
        spec: AssemblySpec,
        operands: Vec<(Intern<String>, crate::typed::ExprId)>,
        tied_output_indices: Vec<Option<usize>>,
        record: crate::typed::ExprId,
    ) -> Self {
        Self {
            target: spec.target,
            dialect: spec.dialect,
            template: spec.template,
            inputs: spec.inputs,
            result: spec.result,
            outputs: spec.outputs,
            clobbers: spec.clobbers,
            memory: spec.memory,
            side_effects: spec.side_effects,
            operands,
            record,
            construction: Vec::new(),
            tied_output_indices,
        }
    }

    fn as_spec(&self) -> AssemblySpec {
        AssemblySpec {
            target: self.target.clone(),
            dialect: self.dialect.clone(),
            inputs: self.inputs.clone(),
            result: self.result.clone(),
            outputs: self.outputs.clone(),
            clobbers: self.clobbers.clone(),
            memory: self.memory.clone(),
            side_effects: self.side_effects,
            template: self.template.clone(),
        }
    }

    pub fn llvm_constraints(&self) -> Vec<String> {
        self.as_spec()
            .llvm_constraints_with_ties(&self.tied_output_indices)
    }

    pub fn canonical_form(&self) -> String {
        format!("{:?}", self.as_spec())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyInput {
    pub parameter: String,
    pub register: String,
    pub tie: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyOutput {
    pub name: String,
    pub register: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AssemblyMemory {
    pub reads: Vec<(String, String)>,
    pub writes: Vec<(String, String)>,
}

impl AssemblySpec {
    fn output_registers(&self) -> Vec<&String> {
        self.result
            .iter()
            .chain(self.outputs.iter().map(|output| &output.register))
            .collect()
    }

    fn llvm_constraints_with_ties(&self, ties: &[Option<usize>]) -> Vec<String> {
        assert_eq!(
            ties.len(),
            self.inputs.len(),
            "checked assembly tie count must match inputs"
        );
        let output_registers = self.output_registers();
        let mut constraints = self
            .result
            .as_ref()
            .map(|register| vec![format!("={{{register}}}")])
            .unwrap_or_default();
        constraints.extend(
            self.outputs
                .iter()
                .map(|output| format!("={{{}}}", output.register)),
        );
        constraints.extend(self.inputs.iter().zip(ties.iter()).map(|(input, tie)| {
            if input.tie {
                let index = tie.expect("checked tied input must identify an output");
                assert_eq!(
                    output_registers.get(index).copied(),
                    Some(&input.register),
                    "checked tied output must match the input register"
                );
                index.to_string()
            } else {
                assert!(
                    tie.is_none(),
                    "checked untied input must not identify an output"
                );
                format!("{{{}}}", input.register)
            }
        }));
        constraints.extend(
            self.clobbers
                .iter()
                .map(|register| format!("~{{{register}}}")),
        );
        if self.side_effects || !self.memory.reads.is_empty() || !self.memory.writes.is_empty() {
            constraints.push("~{memory}".to_string());
        }
        constraints
    }
}

pub(crate) fn populate_package_specifications(
    files: &mut [TypedFileAst],
    target: &flask::CompileTarget,
) {
    let programs: Vec<_> = files.iter().map(TypedFileAst::resolved_program).collect();
    let mut values = Vec::new();
    for (file, program) in programs.iter().copied().enumerate() {
        for kind in &program.exprs.kind {
            let crate::typed::TypedExprKind::Emit { specification, .. } = kind else {
                continue;
            };
            let mut evaluator =
                crate::eval::Evaluator::new(program, crate::eval::CompileTimeHost::new(target))
                    .with_programs(programs.iter().copied());
            if let Ok(crate::eval::EvalOutcome::Complete(value)) = evaluator.eval(*specification) {
                values.push((file, *specification, value));
            }
        }
    }
    for (file, specification, value) in values {
        files[file].exprs.facts.const_facts.const_value[specification.as_usize()] = Some(value);
    }
}

pub fn validate_assembly_expressions(typed: &mut TypedFileAst, target: &flask::CompileTarget) {
    validate_with_programs(typed, target, &[]);
}

pub(crate) fn validate_package_assembly_expressions(
    files: &mut [TypedFileAst],
    target: &flask::CompileTarget,
) {
    for index in 0..files.len() {
        let (before, rest) = files.split_at_mut(index);
        let (typed, after) = rest.split_first_mut().unwrap();
        let programs: Vec<_> = before
            .iter()
            .chain(after.iter())
            .map(TypedFileAst::resolved_program)
            .collect();
        validate_with_programs(typed, target, &programs);
    }
}

fn validate_with_programs(
    typed: &mut TypedFileAst,
    target: &flask::CompileTarget,
    programs: &[crate::ResolvedProgram<'_>],
) {
    populate_target_constants(typed, target);
    let ids: Vec<_> = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .filter_map(|(index, kind)| {
            matches!(kind, crate::typed::TypedExprKind::Emit { .. })
                .then_some(crate::typed::ExprId(index as u32))
        })
        .collect();
    validate_assembly_ids(typed, target, ids, programs);
}

pub(crate) fn target_const_record(target: &flask::TargetTriple) -> ConstValue {
    ConstValue::Record {
        fields: vec![
            (
                Intern::from_ref("arch"),
                ConstValue::Text(target.arch_literal().into_owned().into()),
            ),
            (
                Intern::from_ref("vendor"),
                ConstValue::Text(target.vendor_literal().into()),
            ),
            (
                Intern::from_ref("os"),
                ConstValue::Text(target.os_literal().into_owned().into()),
            ),
            (
                Intern::from_ref("triple"),
                ConstValue::Text(target.raw.clone()),
            ),
        ]
        .into(),
    }
}

pub fn populate_target_constants(typed: &mut TypedFileAst, target: &flask::CompileTarget) {
    if let flask::CompileTarget::Concrete(target) = target {
        let value = target_const_record(target);
        let updates: Vec<(usize, Option<ConstValue>)> = typed
            .exprs
            .kind
            .iter()
            .enumerate()
            .filter_map(|(index, kind)| match kind {
                crate::typed::TypedExprKind::Target => Some((index, Some(value.clone()))),
                crate::typed::TypedExprKind::TargetField(field) => {
                    Some((index, value.get_field(field.as_str()).cloned()))
                }
                _ => None,
            })
            .collect();
        for (index, update) in updates {
            typed.exprs.facts.const_facts.const_value[index] = update;
        }
    }
}

fn validate_assembly_ids(
    typed: &mut TypedFileAst,
    target: &flask::CompileTarget,
    ids: Vec<crate::typed::ExprId>,
    programs: &[crate::ResolvedProgram<'_>],
) {
    for id in ids {
        let (specification, operands, record) = match typed.exprs.kind_of(id).cloned() {
            Some(crate::typed::TypedExprKind::Emit {
                specification,
                operands,
                record,
                ..
            }) => (specification, operands, record),
            _ => continue,
        };
        let mut errors = Vec::new();
        {
            match crate::emission::signature(
                &typed.exprs.ty[specification.as_usize()],
                &typed.type_registry,
            ) {
                Some((input, output)) => {
                    if !matches!(
                        typed.type_registry.resolved_definition_for_type(&input),
                        Ty::Record { .. } | Ty::Sequence(_) | Ty::Unit
                    ) {
                        errors.push("#emit input signature must be a record".into());
                    }
                    if typed.exprs.ty[record.as_usize()] != input {
                        errors.push(
                            "#emit arguments must match the code signature's input type".into(),
                        );
                    }
                    if typed.exprs.ty[id.as_usize()] != output {
                        errors.push(
                            "#emit result must match the code signature's output type".into(),
                        );
                    }
                }
                None => errors.push(
                    "#emit code requires a signature field with input and output type arguments"
                        .into(),
                ),
            }
        }
        let defer_target = matches!(target, flask::CompileTarget::Library);
        let mut supplied_names = HashSet::new();
        for (name, _) in &operands {
            if !supplied_names.insert(name.as_str()) {
                errors.push(format!("duplicate assembly operand `{name}`"));
            }
        }
        let mut construction = Vec::new();
        let mut tied_output_indices = Vec::new();
        let mut checked = if defer_target {
            None
        } else {
            let context = crate::emission::EmissionContext::new(typed.resolved_program(), programs);
            match crate::emission::evaluate(context, id, specification, target) {
                Ok((value, prefix)) => {
                    construction = prefix;
                    let result = crate::emission::compile(
                        &value,
                        &typed.exprs.ty[specification.as_usize()],
                        context,
                        target,
                    );
                    typed.exprs.facts.const_facts.const_value[specification.as_usize()] =
                        Some(value);
                    match result {
                        Ok(spec) => Some(spec),
                        Err(error) => {
                            errors.push(error.message);
                            None
                        }
                    }
                }
                Err(failure) => {
                    if failure.frames.is_empty() {
                        errors.push(failure.diagnostic.message);
                    } else {
                        typed.record_eval_failure(id, *failure);
                    }
                    None
                }
            }
        };
        if let Some(spec) = &mut checked {
            {
                let ty = &typed.exprs.ty[id.as_usize()];
                let fields = crate::emission::operand_fields(ty, &typed.type_registry);
                if fields.is_empty() && *ty != Ty::Unit {
                    if spec.outputs.len() == 1 && spec.outputs[0].name == "value" {
                        let output = spec.outputs.remove(0);
                        validate_assembly_repr(typed, ty, "value", &output.register, &mut errors);
                        spec.result = Some(output.register);
                    } else {
                        errors.push(
                            "scalar #emit result requires exactly one output named `value`".into(),
                        );
                    }
                } else {
                    spec.outputs.sort_by_key(|output| {
                        fields
                            .iter()
                            .position(|(name, _)| name.as_str() == output.name)
                    });
                }
            }
            if !defer_target {
                let declared_results: Vec<_> = crate::emission::operand_fields(
                    &typed.exprs.ty[id.as_usize()],
                    &typed.type_registry,
                )
                .into_iter()
                .map(|(name, ty)| (name.to_string(), ty))
                .collect();
                if spec.result.is_none() {
                    let declared_names = declared_results
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<HashSet<_>>();
                    let mut output_names = HashSet::new();
                    for output in &spec.outputs {
                        if !output_names.insert(output.name.as_str()) {
                            errors.push(format!("duplicate assembly output `{}`", output.name));
                        } else if !declared_names.contains(output.name.as_str()) {
                            errors.push(format!("unknown assembly output `{}`", output.name));
                        }
                    }
                    for (name, _) in declared_results
                        .iter()
                        .filter(|(name, _)| !spec.outputs.iter().any(|output| output.name == *name))
                    {
                        errors.push(format!("missing assembly output `{name}`"));
                    }
                    if spec.outputs.len() != declared_results.len() {
                        errors.push(format!(
                            "assembly output count {} does not match callable result count {}",
                            spec.outputs.len(),
                            declared_results.len()
                        ));
                    }
                    for output in &spec.outputs {
                        let Some((_, ty)) = declared_results
                            .iter()
                            .find(|(name, _)| name == &output.name)
                        else {
                            continue;
                        };
                        validate_assembly_repr(
                            typed,
                            ty,
                            &output.name,
                            &output.register,
                            &mut errors,
                        );
                    }
                }
                for input in &spec.inputs {
                    let Some((_, operand)) = operands
                        .iter()
                        .find(|(name, _)| name.as_str() == input.parameter)
                    else {
                        continue;
                    };
                    match crate::representation::Repr::derive(
                        &typed.exprs.ty[operand.as_usize()],
                        Some(&typed.type_registry),
                    ) {
                        Ok(crate::representation::Repr::Bits { width }) => {
                            if let Some(expected) = register_width(&input.register)
                                && width != expected
                            {
                                errors.push(format!(
                                    "assembly operand `{}` has {width}-bit representation, incompatible with register `{}` ({expected} bits)",
                                    input.parameter, input.register
                                ));
                            }
                        }
                        Ok(crate::representation::Repr::Address { .. }) => {
                            validate_address_register(typed, &input.parameter, &input.register, &mut errors);
                        }
                        Ok(repr) => errors.push(format!(
                            "assembly operand `{}` has unsupported physical representation `{repr:?}`",
                            input.parameter
                        )),
                        Err(error) => errors.push(format!(
                            "assembly operand `{}` has no physical representation: {error}",
                            input.parameter
                        )),
                    }
                }
                for (pointer, length) in spec.memory.reads.iter().chain(spec.memory.writes.iter()) {
                    let pointer_ty = operands
                        .iter()
                        .find(|(name, _)| name.as_str() == pointer)
                        .map(|(_, expr)| &typed.exprs.ty[expr.as_usize()]);
                    let length_ty = operands
                        .iter()
                        .find(|(name, _)| name.as_str() == length)
                        .map(|(_, expr)| &typed.exprs.ty[expr.as_usize()]);
                    if !pointer_ty.is_some_and(|ty| {
                        matches!(
                            crate::representation::Repr::derive(ty, Some(&typed.type_registry)),
                            Ok(crate::representation::Repr::Address { .. })
                        )
                    }) {
                        errors.push(format!(
                            "memory pointer `{pointer}` must have Address representation"
                        ));
                    }
                    if !length_ty.is_some_and(|ty| {
                        matches!(
                            crate::representation::Repr::derive(ty, Some(&typed.type_registry)),
                            Ok(crate::representation::Repr::Bits { width: 64 })
                        )
                    }) {
                        errors.push(format!(
                            "memory length `{length}` must have 64-bit Bits representation"
                        ));
                    }
                }
            }
            if let flask::CompileTarget::Concrete(active) = target {
                if spec.target != active.raw {
                    errors.push(format!(
                        "assembly target `{}` does not match active target `{}`",
                        spec.target, active.raw
                    ));
                }
                let supported = match active.architecture() {
                    flask::Architecture::X86_64 => matches!(spec.dialect.as_str(), "att" | "intel"),
                    flask::Architecture::Aarch64(_) => spec.dialect == "att",
                    flask::Architecture::Wasm32 => false,
                    _ => false,
                };
                if !supported {
                    errors.push(format!(
                        "assembly dialect `{}` is unsupported for target `{}`",
                        spec.dialect, active.raw
                    ));
                }
                for register in spec
                    .inputs
                    .iter()
                    .map(|input| input.register.as_str())
                    .chain(spec.result.iter().map(String::as_str))
                    .chain(spec.outputs.iter().map(|output| output.register.as_str()))
                {
                    if !valid_register(active.architecture(), register) {
                        errors.push(format!(
                            "assembly register `{register}` is unsupported for target `{}`",
                            active.raw
                        ));
                    }
                }
                for register in spec.clobbers.iter().map(String::as_str) {
                    if !valid_clobber(active.architecture(), register) {
                        errors.push(format!(
                            "assembly register `{register}` is unsupported for target `{}`",
                            active.raw
                        ));
                    }
                }
            } else {
                errors.push(
                    "target-specific assembly requires a concrete compilation target".to_string(),
                );
            }
            let supplied: HashSet<_> = operands.iter().map(|(name, _)| name.as_str()).collect();
            let required: HashSet<_> = spec
                .inputs
                .iter()
                .map(|input| input.parameter.as_str())
                .collect();
            for name in required.difference(&supplied) {
                errors.push(format!("missing assembly operand `{name}`"));
            }
            for name in supplied.difference(&required) {
                errors.push(format!("extra assembly operand `{name}`"));
            }
            for (pointer, length) in spec.memory.reads.iter().chain(spec.memory.writes.iter()) {
                if !supplied.contains(pointer.as_str()) {
                    errors.push(format!("unknown memory pointer parameter `{pointer}`"));
                }
                if !supplied.contains(length.as_str()) {
                    errors.push(format!("unknown memory length parameter `{length}`"));
                }
            }
            let mut registers = HashSet::new();
            for register in spec
                .inputs
                .iter()
                .filter(|input| !input.tie)
                .map(|input| &input.register)
                .chain(spec.result.iter())
                .chain(spec.outputs.iter().map(|output| &output.register))
                .chain(spec.clobbers.iter())
            {
                if !registers.insert(register) {
                    errors.push(format!("duplicate assembly register binding `{register}`"));
                }
            }
            if let Some(result) = &spec.result {
                for input in &spec.inputs {
                    if input.tie && input.register != *result {
                        errors.push(format!(
                            "tied input `{}` must use result register `{result}`",
                            input.parameter
                        ));
                    }
                }
            } else if !spec.outputs.is_empty() {
                for input in spec.inputs.iter().filter(|input| input.tie) {
                    if !spec
                        .outputs
                        .iter()
                        .any(|output| output.register == input.register)
                    {
                        errors.push(format!(
                            "tied input `{}` must reference a declared output register",
                            input.parameter
                        ));
                    }
                }
            } else if spec.inputs.iter().any(|input| input.tie) {
                errors.push("a tied input requires a result register".to_string());
            }
            let output_registers = spec.output_registers();
            let mut tied_outputs = HashSet::new();
            tied_output_indices = spec
                .inputs
                .iter()
                .map(|input| {
                    if !input.tie {
                        return None;
                    }
                    let index = output_registers
                        .iter()
                        .position(|register| *register == &input.register)?;
                    if !tied_outputs.insert(index) {
                        errors.push(format!(
                            "multiple tied inputs target assembly output {index}"
                        ));
                    }
                    Some(index)
                })
                .collect::<Vec<_>>();
            let result_is_unit = matches!(typed.exprs.ty[id.as_usize()], Ty::Unit);
            let has_result = spec.result.is_some() || !spec.outputs.is_empty();
            if has_result == result_is_unit {
                errors.push(if has_result {
                    "assembly result requires a contextual non-Unit type".to_string()
                } else {
                    "assembly without a result has type Unit".to_string()
                });
            }
        }
        let flaws = errors
            .iter()
            .map(|message| {
                // type-invalid-assembly
                Diagnostic::new(EmissionCode::AssemblyInvalid, message.clone())
                    .at_span_id(typed.exprs.span[id.as_usize()], &typed.span_table)
            })
            .collect::<Vec<_>>();
        if let Some(crate::typed::TypedExprKind::Emit { .. }) = typed.exprs.kind_of(id) {
            typed.exprs.facts.semantic_facts.flaws[id.as_usize()].extend(flaws);
            let state = if defer_target {
                AssemblyState::Pending
            } else if errors.is_empty() {
                checked
                    .map(|spec| {
                        let resolved_operands = spec
                            .inputs
                            .iter()
                            .filter_map(|input| {
                                operands
                                    .iter()
                                    .find(|(name, _)| name.as_str() == input.parameter)
                                    .cloned()
                            })
                            .collect();
                        let mut assembly = CheckedAssembly::from_spec(
                            spec,
                            resolved_operands,
                            tied_output_indices,
                            record,
                        );
                        assembly.construction = construction;
                        AssemblyState::Checked(Box::new(assembly))
                    })
                    .unwrap_or(AssemblyState::Invalid)
            } else {
                AssemblyState::Invalid
            };
            if let Some(crate::typed::TypedExprKind::Emit { state: slot, .. }) =
                typed.exprs.kind.get_mut(id.as_usize())
            {
                *slot = state;
            }
        }
    }
}

/// x86-64 general-purpose registers in encoding index order (0 = rax, 4 = rsp).
pub(super) const X86_64_GENERAL_REGISTERS: [&str; 16] = [
    "rax", "rcx", "rdx", "rbx", "rsp", "rbp", "rsi", "rdi", "r8", "r9", "r10", "r11", "r12", "r13",
    "r14", "r15",
];

fn is_x86_64_general_register(register: &str) -> bool {
    X86_64_GENERAL_REGISTERS.contains(&register)
}

fn valid_register(arch: flask::Architecture, register: &str) -> bool {
    match arch {
        flask::Architecture::X86_64 => is_x86_64_general_register(register),
        flask::Architecture::Aarch64(_) => {
            register == "sp"
                || register
                    .strip_prefix(['x', 'w'])
                    .is_some_and(|n| n.parse::<u8>().is_ok_and(|index| index <= 30))
        }
        flask::Architecture::Wasm32 => false,
        _ => false,
    }
}

fn valid_clobber(arch: flask::Architecture, register: &str) -> bool {
    valid_register(arch, register)
        || (matches!(register, "memory" | "cc") && !matches!(arch, flask::Architecture::Wasm32))
}

fn validate_assembly_repr(
    typed: &TypedFileAst,
    ty: &Ty,
    name: &str,
    register: &str,
    errors: &mut Vec<String>,
) {
    let Ok(repr) = crate::representation::Repr::derive(ty, Some(&typed.type_registry)) else {
        errors.push(format!(
            "assembly output `{name}` has no physical representation"
        ));
        return;
    };
    if let crate::representation::Repr::Bits { width } = repr {
        if let Some(expected) = register_width(register)
            && width != expected
        {
            errors.push(format!(
                "assembly output `{name}` has {width}-bit representation, incompatible with register `{register}` ({expected} bits)"
            ));
        }
    } else if matches!(repr, crate::representation::Repr::Address { .. }) {
        validate_address_register(typed, name, register, errors);
    } else {
        errors.push(format!(
            "assembly output `{name}` has unsupported physical representation"
        ));
    }
}

fn validate_address_register(
    typed: &TypedFileAst,
    name: &str,
    register: &str,
    errors: &mut Vec<String>,
) {
    let width = typed.target_layout.map(|layout| layout.pointer_bits());
    if width != register_width(register).map(u64::from) {
        errors.push(format!(
            "assembly address `{name}` requires a register with the target pointer width"
        ));
    }
}

fn register_width(register: &str) -> Option<u32> {
    let name = register.strip_prefix('%').unwrap_or(register);
    if matches!(name, "sp" | "xzr" | "wzr") {
        return Some(if name.starts_with('w') { 32 } else { 64 });
    }
    if let Some(number) = name.strip_prefix('x')
        && number.parse::<u8>().is_ok_and(|value| value <= 30)
    {
        return Some(64);
    }
    if let Some(number) = name.strip_prefix('w')
        && number.parse::<u8>().is_ok_and(|value| value <= 30)
    {
        return Some(32);
    }
    if is_x86_64_general_register(name) {
        return Some(64);
    }
    if matches!(
        name,
        "eax" | "ebx" | "ecx" | "edx" | "esi" | "edi" | "ebp" | "esp"
    ) {
        return Some(32);
    }
    None
}

#[cfg(test)]
#[path = "assembly_constraints_tests.rs"]
mod constraints_tests;
