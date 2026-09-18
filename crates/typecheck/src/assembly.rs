use internment::Intern;
use std::collections::HashSet;

use crate::subst::DepSubst;
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
    Checked(CheckedAssembly),
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckedAssembly {
    pub evaluation: Option<crate::DefId>,
    pub target: String,
    pub dialect: String,
    pub template: String,
    pub inputs: Vec<AssemblyInput>,
    pub operands: Vec<(Intern<String>, crate::typed::ExprId)>,
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
    ) -> Self {
        Self {
            evaluation: None,
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
        self.as_spec().canonical_form()
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyParseError {
    pub line: usize,
    pub message: String,
}

impl AssemblySpec {
    pub fn with_output(mut self, name: impl Into<String>, register: impl Into<String>) -> Self {
        self.outputs.push(AssemblyOutput {
            name: name.into(),
            register: register.into(),
        });
        self
    }

    pub fn canonical_form(&self) -> String {
        let mut result = format!(
            "gin-asm 1\ntarget \"{}\"\ndialect {}\n",
            self.target, self.dialect
        );
        for input in &self.inputs {
            result.push_str(&format!(
                "input \"{}\" {} \"{}\"\n",
                input.parameter,
                if input.tie { "tied" } else { "fixed" },
                input.register
            ));
        }
        if let Some(register) = &self.result {
            result.push_str(&format!("result fixed \"{register}\"\n"));
        }
        for output in &self.outputs {
            result.push_str(&format!(
                "output \"{}\" fixed \"{}\"\n",
                output.name, output.register
            ));
        }
        for register in &self.clobbers {
            result.push_str(&format!("clobber \"{register}\"\n"));
        }
        for (pointer, length) in &self.memory.reads {
            result.push_str(&format!(
                "memory read pointer=\"{pointer}\" length=\"{length}\"\n"
            ));
        }
        for (pointer, length) in &self.memory.writes {
            result.push_str(&format!(
                "memory write pointer=\"{pointer}\" length=\"{length}\"\n"
            ));
        }
        if self.side_effects {
            result.push_str("option side_effects\n");
        }
        result.push_str(&format!("template \"{}\"", self.template));
        result
    }

    pub fn llvm_constraints(&self) -> Vec<String> {
        let ties = self
            .inputs
            .iter()
            .map(|input| {
                if !input.tie {
                    return None;
                }
                self.output_registers()
                    .iter()
                    .position(|register| *register == &input.register)
            })
            .collect::<Vec<_>>();
        self.llvm_constraints_with_ties(&ties)
    }

    fn output_registers(&self) -> Vec<&String> {
        self.result
            .iter()
            .chain(self.outputs.iter().map(|output| &output.register))
            .collect()
    }

    fn llvm_constraints_with_ties(&self, ties: &[Option<usize>]) -> Vec<String> {
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
                tie.map(|index| index.to_string())
                    .unwrap_or_else(|| "0".to_string())
            } else {
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

    pub fn parse(source: &str) -> Result<Self, Vec<AssemblyParseError>> {
        let mut version = None;
        let mut target = None;
        let mut dialect = None;
        let mut inputs = Vec::new();
        let mut result = None;
        let mut outputs = Vec::new();
        let mut clobbers = Vec::new();
        let mut memory = AssemblyMemory::default();
        let mut side_effects = true;
        let mut template = None;
        let mut errors = Vec::new();
        let mut seen = HashSet::new();

        for (line_index, line) in source.lines().enumerate() {
            let line_number = line_index + 1;
            let mut fields = line.splitn(2, ' ');
            let Some(kind) = fields.next() else { continue };
            let rest = fields.next().unwrap_or_default().trim();
            if kind.is_empty() {
                continue;
            }
            let duplicate =
                !matches!(kind, "input" | "output" | "clobber" | "memory") && !seen.insert(kind);
            if duplicate {
                errors.push(AssemblyParseError {
                    line: line_number,
                    message: format!("duplicate `{kind}` field"),
                });
                continue;
            }
            match kind {
                "gin-asm" => version = rest.parse::<u32>().ok(),
                "target" => target = quoted(rest),
                "dialect" => dialect = Some(rest.to_string()),
                "input" => match parse_input(rest) {
                    Some(input) => inputs.push(input),
                    None => errors.push(error(
                        line_number,
                        "expected `input \"name\" fixed \"register\"`",
                    )),
                },
                "result" => match parse_fixed(rest) {
                    Some(register) => result = Some(register),
                    None => errors.push(error(line_number, "expected `result fixed \"register\"`")),
                },
                "output" => match parse_output(rest) {
                    Some(output) => outputs.push(output),
                    None => errors.push(error(
                        line_number,
                        "expected `output \"name\" fixed \"register\"`",
                    )),
                },
                "clobber" => match quoted(rest) {
                    Some(register) => clobbers.push(register),
                    None => errors.push(error(line_number, "expected quoted clobber register")),
                },
                "memory" => match parse_memory(rest) {
                    Some((kind, pointer, length)) => match kind {
                        "read" => memory.reads.push((pointer, length)),
                        "write" => memory.writes.push((pointer, length)),
                        _ => errors.push(error(
                            line_number,
                            "memory effect must be `read` or `write`",
                        )),
                    },
                    None => errors.push(error(
                        line_number,
                        "expected `memory read|write pointer=\"p\" length=\"n\"`",
                    )),
                },
                "option" if rest == "side_effects" => side_effects = true,
                "template" => template = quoted_template(rest),
                _ => errors.push(error(
                    line_number,
                    format!("unknown assembly field `{kind}`"),
                )),
            }
        }

        if version != Some(1) {
            errors.push(error(1, "unsupported or missing `gin-asm 1` version"));
        }
        if target.is_none() {
            errors.push(error(1, "missing target"));
        }
        if dialect.is_none() {
            errors.push(error(1, "missing dialect"));
        }
        if template.is_none() {
            errors.push(error(1, "missing template"));
        }
        if errors.is_empty() {
            Ok(Self {
                target: target.unwrap(),
                dialect: dialect.unwrap(),
                inputs,
                result,
                outputs,
                clobbers,
                memory,
                side_effects,
                template: template.unwrap(),
            })
        } else {
            Err(errors)
        }
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
            let crate::typed::TypedExprKind::Asm { specification, .. } = kind else {
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
    populate_target_constants(typed, target);
    let ids: Vec<_> = typed
        .exprs
        .kind
        .iter()
        .enumerate()
        .filter_map(|(index, kind)| {
            matches!(kind, crate::typed::TypedExprKind::Asm { .. })
                .then_some(crate::typed::ExprId(index as u32))
        })
        .collect();
    validate_assembly_ids(typed, target, ids);
}

pub fn populate_target_constants(typed: &mut TypedFileAst, target: &flask::CompileTarget) {
    if let flask::CompileTarget::Concrete(target) = target {
        let value = ConstValue::Record {
            fields: vec![
                (
                    Intern::from_ref("arch"),
                    ConstValue::Text(target.arch.as_str().into()),
                ),
                (
                    Intern::from_ref("vendor"),
                    ConstValue::Text(target.vendor.clone()),
                ),
                (
                    Intern::from_ref("os"),
                    ConstValue::Text(target.os.as_str().into()),
                ),
                (
                    Intern::from_ref("triple"),
                    ConstValue::Text(target.raw.clone()),
                ),
            ]
            .into(),
        };
        for (index, kind) in typed.exprs.kind.iter().enumerate() {
            if matches!(kind, crate::typed::TypedExprKind::Target) {
                typed.exprs.facts.const_facts.const_value[index] = Some(value.clone());
            }
        }
        for (index, kind) in typed.exprs.kind.iter().enumerate() {
            let crate::typed::TypedExprKind::TargetField(field) = kind else {
                continue;
            };
            typed.exprs.facts.const_facts.const_value[index] =
                value.get_field(field.as_str()).cloned();
        }
    }
}

fn validate_assembly_ids(
    typed: &mut TypedFileAst,
    target: &flask::CompileTarget,
    ids: Vec<crate::typed::ExprId>,
) {
    for id in ids {
        let (specification, operands) = match typed.exprs.kind_of(id).cloned() {
            Some(crate::typed::TypedExprKind::Asm {
                specification,
                operands,
                ..
            }) => (specification, operands),
            _ => continue,
        };
        let mut errors = Vec::new();
        let defer_target = matches!(target, flask::CompileTarget::Library);
        let mut supplied_names = HashSet::new();
        for (name, _) in &operands {
            if !supplied_names.insert(name.as_str()) {
                errors.push(format!("duplicate assembly operand `{name}`"));
            }
        }
        let normalized =
            (!defer_target).then(|| evaluate_specification(typed, specification, target));
        let mut tied_output_indices = Vec::new();
        let mut evaluation = None;
        let checked = match normalized {
            None => None,
            Some(crate::staging::NormalizeResult::Value(
                value @ (ConstValue::Record { .. } | ConstValue::Tag { .. }),
            )) => {
                let leaf = value.get_field("machine_leaf").is_some();
                let machine = leaf || value.get_field("machine_backend").is_some();
                let result = if leaf {
                    crate::machine::backend::compile_leaf(&value, typed.resolved_program(), target)
                        .map(|(spec, method)| {
                            evaluation = method;
                            spec
                        })
                } else if machine {
                    crate::machine::backend::compile(&value, typed.resolved_program(), target)
                } else {
                    crate::encoding::EncodingRegion::from_value(&value, typed.resolved_program())
                        .and_then(|region| {
                            evaluation = region.evaluation_method(typed.resolved_program())?;
                            region
                                .compile(typed.resolved_program(), target)
                                .map(|region| region.into_assembly())
                        })
                };
                match result {
                    Ok(spec) => {
                        if machine && evaluation.is_none() {
                            let source = spec.canonical_form();
                            typed.exprs.kind[specification.as_usize()] =
                                crate::typed::TypedExprKind::Lit(ast::Literal::String(
                                    source.clone(),
                                ));
                            typed.exprs.ty[specification.as_usize()] =
                                Ty::Literal(ConstValue::Text(source.clone()));
                            typed.exprs.facts.const_facts.const_value[specification.as_usize()] =
                                Some(ConstValue::Text(source));
                        }
                        Some(spec)
                    }
                    Err(error) => {
                        typed.exprs.facts.semantic_facts.flaws[id.as_usize()].push(
                            error.at_span_id(
                                typed.exprs.span[specification.as_usize()],
                                &typed.span_table,
                            ),
                        );
                        None
                    }
                }
            }
            Some(crate::staging::NormalizeResult::Value(value)) => {
                if let Some(source) = value.as_utf8() {
                    if source.trim().is_empty() {
                        errors.push(
                            "no checked assembly implementation is available for the active target"
                                .to_string(),
                        );
                        None
                    } else {
                        match AssemblySpec::parse(&source) {
                            Ok(spec) => Some(spec),
                            Err(parse_errors) => {
                                errors.extend(parse_errors.into_iter().map(|e| e.message));
                                None
                            }
                        }
                    }
                } else {
                    errors.push("assembly specification must be compile-time UTF-8 bytes or an instruction region".to_string());
                    None
                }
            }
            Some(crate::staging::NormalizeResult::Residual(_)) => {
                let rendered = match typed.exprs.kind_of(specification) {
                    _ => evaluate_builder_call(typed, specification, target),
                };
                match rendered {
                    Some(source) => match AssemblySpec::parse(&source) {
                        Ok(spec) => Some(spec),
                        Err(parse_errors) => {
                            errors.extend(parse_errors.into_iter().map(|error| error.message));
                            None
                        }
                    },
                    None => {
                        errors.push("could not evaluate compile-time assembly builder".to_string());
                        None
                    }
                }
            }
            Some(_) => {
                errors.push(
                    "assembly specification must be compile-time text or an instruction region"
                        .to_string(),
                );
                None
            }
        };
        if let Some(spec) = &checked {
            if spec.result.is_some() && !spec.outputs.is_empty() {
                errors.push("assembly cannot mix legacy result with ordered outputs".to_string());
            }
            if !defer_target {
                let declared_results = match &typed.exprs.ty[id.as_usize()] {
                    Ty::ResultRow(fields) => fields
                        .iter()
                        .map(|field| (field.name.as_str().to_string(), (*field.ty).clone()))
                        .collect::<Vec<_>>(),
                    Ty::Unit => Vec::new(),
                    ty => typed
                        .defs
                        .values()
                        .find_map(|bind| {
                            (matches!(bind.body, crate::typed::BindBody::Expr(expr) if expr == id))
                                .then(|| bind.named_results.first())
                                .flatten()
                                .map(|result| vec![(result.name.as_str().to_string(), ty.clone())])
                        })
                        .unwrap_or_default(),
                };
                if !spec.outputs.is_empty() {
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
                        Ok(crate::representation::Repr::Address { .. }) => {}
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
                let supported = match active.arch {
                    flask::TargetArch::X86_64 => matches!(spec.dialect.as_str(), "att" | "intel"),
                    flask::TargetArch::Arm64 => spec.dialect == "att",
                    flask::TargetArch::Wasm32 => false,
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
                    if !valid_register(active.arch, register) {
                        errors.push(format!(
                            "assembly register `{register}` is unsupported for target `{}`",
                            active.raw
                        ));
                    }
                }
                for register in spec.clobbers.iter().map(String::as_str) {
                    if !valid_clobber(active.arch, register) {
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
                    let Some(index) = output_registers
                        .iter()
                        .position(|register| *register == &input.register)
                    else {
                        return None;
                    };
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
            if let Some(method) = evaluation {
                let bind = &typed.defs[&method];
                if matches!(bind.body, crate::typed::BindBody::Unlowered)
                    || !spec.memory.reads.is_empty()
                    || !spec.memory.writes.is_empty()
                {
                    errors.push("compile-time assembly evaluation requires a Gin method and a contract without memory access".into());
                }
                if bind.return_type != typed.exprs.ty[id.as_usize()] {
                    errors.push(
                        "assembly evaluation method must return the assembly result type".into(),
                    );
                }
                for (input, (_, param)) in spec.inputs.iter().zip(&bind.params) {
                    if let Some((_, operand)) = operands
                        .iter()
                        .find(|(name, _)| name.as_str() == input.parameter)
                        && typed.exprs.ty_of(*operand).is_none_or(|ty| {
                            typed.type_registry.resolved_definition_for_type(ty)
                                != typed.type_registry.resolved_definition_for_type(param)
                        })
                    {
                        errors.push(format!(
                            "assembly evaluation parameter for `{}` must match its operand type",
                            input.parameter
                        ));
                    }
                }
            }
        }
        let flaws = errors
            .iter()
            .map(|message| {
                Diagnostic::new("type-invalid-assembly", message.clone())
                    .at_span_id(typed.exprs.span[id.as_usize()], &typed.span_table)
            })
            .collect::<Vec<_>>();
        if let Some(crate::typed::TypedExprKind::Asm { .. }) = typed.exprs.kind_of(id) {
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
                        );
                        assembly.evaluation = evaluation;
                        AssemblyState::Checked(assembly)
                    })
                    .unwrap_or(AssemblyState::Invalid)
            } else {
                AssemblyState::Invalid
            };
            if let Some(crate::typed::TypedExprKind::Asm { state: slot, .. }) =
                typed.exprs.kind.get_mut(id.as_usize())
            {
                *slot = state;
            }
        }
    }
}

fn valid_register(arch: flask::TargetArch, register: &str) -> bool {
    match arch {
        flask::TargetArch::X86_64 => matches!(
            register,
            "rax"
                | "rbx"
                | "rcx"
                | "rdx"
                | "rsi"
                | "rdi"
                | "rbp"
                | "rsp"
                | "r8"
                | "r9"
                | "r10"
                | "r11"
                | "r12"
                | "r13"
                | "r14"
                | "r15"
        ),
        flask::TargetArch::Arm64 => {
            register == "sp"
                || register
                    .strip_prefix('x')
                    .is_some_and(|n| n.parse::<u8>().is_ok_and(|index| index <= 30))
        }
        flask::TargetArch::Wasm32 => false,
    }
}

fn valid_clobber(arch: flask::TargetArch, register: &str) -> bool {
    valid_register(arch, register)
        || (matches!(register, "memory" | "cc") && !matches!(arch, flask::TargetArch::Wasm32))
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
    } else if !matches!(repr, crate::representation::Repr::Address { .. }) {
        errors.push(format!(
            "assembly output `{name}` has unsupported physical representation"
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
    if matches!(
        name,
        "rax"
            | "rbx"
            | "rcx"
            | "rdx"
            | "rsi"
            | "rdi"
            | "rbp"
            | "rsp"
            | "r8"
            | "r9"
            | "r10"
            | "r11"
            | "r12"
            | "r13"
            | "r14"
            | "r15"
    ) {
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

fn evaluate_builder_call(
    typed: &TypedFileAst,
    specification: crate::typed::ExprId,
    target: &flask::CompileTarget,
) -> Option<String> {
    let mut evaluator = crate::eval::Evaluator::new(
        typed.resolved_program(),
        crate::eval::CompileTimeHost::new(target),
    );
    match evaluator.eval(specification).ok()? {
        crate::eval::EvalOutcome::Complete(value) => value.as_utf8().map(|text| text.into_owned()),
        crate::eval::EvalOutcome::Address(_)
        | crate::eval::EvalOutcome::Residual(_)
        | crate::eval::EvalOutcome::RuntimeDependency(_)
        | crate::eval::EvalOutcome::Trap(_)
        | crate::eval::EvalOutcome::Cancelled => None,
    }
}

fn evaluate_specification(
    typed: &TypedFileAst,
    specification: crate::typed::ExprId,
    target: &flask::CompileTarget,
) -> crate::staging::NormalizeResult {
    if let Some(Some(value)) = typed.exprs.const_value_of(specification) {
        return crate::staging::NormalizeResult::Value(value.clone());
    }
    let mut evaluator = crate::eval::Evaluator::new(
        typed.resolved_program(),
        crate::eval::CompileTimeHost::new(target),
    );
    match evaluator.eval(specification) {
        Ok(crate::eval::EvalOutcome::Complete(value)) => {
            crate::staging::NormalizeResult::Value(value)
        }
        Ok(crate::eval::EvalOutcome::RuntimeDependency(_))
        | Ok(crate::eval::EvalOutcome::Residual(_))
        | Ok(crate::eval::EvalOutcome::Cancelled)
        | Ok(crate::eval::EvalOutcome::Address(_))
        | Ok(crate::eval::EvalOutcome::Trap(_))
        | Err(_) => crate::staging::normalize(typed, specification, &DepSubst::new()),
    }
}

fn error(line: usize, message: impl Into<String>) -> AssemblyParseError {
    AssemblyParseError {
        line,
        message: message.into(),
    }
}

fn quoted(value: &str) -> Option<String> {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .map(ToOwned::to_owned)
}

fn quoted_template(value: &str) -> Option<String> {
    Some(quoted(value)?.replace("\\n", "\n"))
}

fn parse_fixed(value: &str) -> Option<String> {
    let mut fields = value.splitn(2, ' ');
    (fields.next()? == "fixed")
        .then(|| quoted(fields.next()?.trim()))
        .flatten()
}

fn parse_output(value: &str) -> Option<AssemblyOutput> {
    let name = value.strip_prefix('"')?.split_once('"')?;
    let rest = name.1.trim();
    Some(AssemblyOutput {
        name: name.0.to_string(),
        register: parse_fixed(rest)?,
    })
}

fn parse_input(value: &str) -> Option<AssemblyInput> {
    let parameter = value.strip_prefix('"')?.split_once('"')?;
    let rest = parameter.1.trim();
    let mut fields = rest.splitn(2, ' ');
    let binding = fields.next()?;
    if !matches!(binding, "fixed" | "tied") {
        return None;
    }
    Some(AssemblyInput {
        parameter: parameter.0.to_string(),
        register: quoted(fields.next()?.trim())?,
        tie: binding == "tied",
    })
}

fn parse_memory(value: &str) -> Option<(&str, String, String)> {
    let mut fields = value.split_whitespace();
    let kind = fields.next()?;
    let pointer = fields.next()?.strip_prefix("pointer=").and_then(quoted)?;
    let length = fields.next()?.strip_prefix("length=").and_then(quoted)?;
    (fields.next().is_none()).then_some((kind, pointer, length))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versioned_assembly_protocol() {
        let spec = AssemblySpec::parse("gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"fd\" fixed \"rdi\"\nresult fixed \"rax\"\nclobber \"rcx\"\nmemory read pointer=\"buf\" length=\"len\"\noption side_effects\ntemplate \"syscall\"").unwrap();
        assert_eq!(spec.inputs[0].parameter, "fd");
        assert_eq!(spec.result.as_deref(), Some("rax"));
        assert!(spec.side_effects);
    }

    #[test]
    fn parses_ordered_named_outputs_and_constraints() {
        let spec = AssemblySpec::parse(
            "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\noutput \"sum\" fixed \"rax\"\noutput \"overflow\" fixed \"rdx\"\ntemplate \"add\"",
        )
        .unwrap();
        assert_eq!(
            spec.outputs,
            vec![
                AssemblyOutput {
                    name: "sum".into(),
                    register: "rax".into()
                },
                AssemblyOutput {
                    name: "overflow".into(),
                    register: "rdx".into()
                },
            ]
        );
        assert_eq!(
            spec.llvm_constraints(),
            vec!["={rax}", "={rdx}", "~{memory}"]
        );
    }

    #[test]
    fn builder_appends_ordered_outputs() {
        let spec = AssemblySpec {
            target: "x86_64-unknown-linux-gnu".into(),
            dialect: "intel".into(),
            inputs: Vec::new(),
            result: None,
            outputs: Vec::new(),
            clobbers: Vec::new(),
            memory: AssemblyMemory::default(),
            side_effects: false,
            template: "add".into(),
        }
        .with_output("sum", "rax")
        .with_output("overflow", "rdx");

        assert_eq!(
            spec.outputs,
            vec![
                AssemblyOutput {
                    name: "sum".into(),
                    register: "rax".into()
                },
                AssemblyOutput {
                    name: "overflow".into(),
                    register: "rdx".into()
                },
            ]
        );
        assert_eq!(spec.llvm_constraints(), vec!["={rax}", "={rdx}"]);
    }

    #[test]
    fn rejects_unknown_fields_and_unsupported_versions_together() {
        let errors = AssemblySpec::parse("gin-asm 2\nunknown value\ntemplate \"x\"").unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("unknown assembly field"))
        );
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("unsupported"))
        );
    }

    #[test]
    fn preserves_explicit_input_result_ties_without_llvm_ordering() {
        let spec = AssemblySpec::parse(
            "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"value\" tied \"rax\"\nresult fixed \"rax\"\ntemplate \"inc rax\"",
        )
        .unwrap();
        assert!(spec.inputs[0].tie);
        assert_eq!(spec.inputs[0].register, "rax");
        assert!(spec.side_effects);
        assert_eq!(spec.llvm_constraints(), vec!["={rax}", "0", "~{memory}"]);
        assert!(
            spec.canonical_form()
                .contains("input \"value\" tied \"rax\"")
        );
    }

    #[test]
    fn resolves_ties_to_ordered_output_indices() {
        let spec = AssemblySpec::parse(
            "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ninput \"lhs\" tied \"rax\"\ninput \"rhs\" tied \"rdx\"\noutput \"sum\" fixed \"rax\"\noutput \"carry\" fixed \"rdx\"\ntemplate \"add\"",
        )
        .unwrap();

        assert_eq!(
            spec.llvm_constraints(),
            vec!["={rax}", "={rdx}", "0", "1", "~{memory}"]
        );
    }

    #[test]
    fn preserves_escaped_newlines_in_real_assembly_templates() {
        let spec = AssemblySpec::parse(
            "gin-asm 1\ntarget \"x86_64-unknown-linux-gnu\"\ndialect intel\ntemplate \"mov rax, 1\\nsyscall\"",
        )
        .unwrap();
        assert_eq!(spec.template, "mov rax, 1\nsyscall");
    }
}
