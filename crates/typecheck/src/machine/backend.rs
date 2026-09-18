use std::borrow::Cow;
use std::collections::{BTreeSet, HashMap};

use ast::ConstValue;
use diagnostic::Diagnostic;
use flask::CompileTarget;

use crate::DefId;
use crate::assembly::AssemblySpec;
use crate::encoding::{EncodingRegion, Fields};
use crate::resolved_program::ResolvedProgram;

pub(crate) fn compile(
    value: &ConstValue,
    program: ResolvedProgram<'_>,
    target: &CompileTarget,
) -> Result<AssemblySpec, Diagnostic> {
    let plan = Fields::new(
        value,
        program,
        &[
            "machine_backend",
            "backends",
            "contract",
            "instructions",
            "registers",
        ],
    )?;
    let backend = read_backend(plan.value("machine_backend"), program, target)?;
    for helper in plan.sequence("backends")? {
        if helper != plan.value("machine_backend") {
            return Err(error("machine helpers must use the caller's backend"));
        }
    }
    let mut spec = AssemblySpec::parse(&plan.text("contract")?)
        .map_err(|_| error("invalid generated machine contract"))?;
    spec.dialect = backend.text("dialect")?.into();
    let mut roles = HashMap::new();
    for (field, prefix) in [("inputs", "$input"), ("references", "$reference")] {
        for (index, value) in backend.sequence(field)?.iter().enumerate() {
            roles.insert(format!("{prefix}{index}"), text(value)?.to_string());
        }
    }
    roles.insert("$result".into(), backend.text("result")?.into());
    roles.insert("$length".into(), backend.text("length")?.into());
    let scratch = backend
        .sequence("scratch")?
        .iter()
        .map(|value| text(value).map(|text| text.into_owned()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    let declared = plan
        .sequence("registers")?
        .iter()
        .map(|value| text(value).map(|text| text.into_owned()))
        .collect::<Result<BTreeSet<_>, _>>()?;
    for register in &declared {
        if !scratch.contains(register) {
            return Err(error(format!(
                "`{register}` is not an available scratch register for this #machine backend"
            )));
        }
    }
    let mut used = BTreeSet::new();
    for input in &spec.inputs {
        used.insert(input.register.clone());
    }
    for register in spec.result.iter().chain(&spec.clobbers) {
        used.insert(register.clone());
    }
    let instructions = plan
        .sequence("instructions")?
        .iter()
        .map(|value| {
            Fields::new(
                value,
                program,
                &["operation", "destination", "left", "right", "boundary"],
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    for instruction in &instructions {
        for operand in ["destination", "left", "right"] {
            used.insert(instruction.text(operand)?.into());
        }
    }
    let resolve = |name: &str| -> Result<String, Diagnostic> {
        if name.starts_with('$') {
            roles
                .get(name)
                .cloned()
                .ok_or_else(|| error(format!("machine backend is missing register role `{name}`")))
        } else {
            Ok(name.into())
        }
    };
    let mut reserved = BTreeSet::new();
    for role in used.iter().filter(|name| name.starts_with('$')) {
        let register = resolve(role)?;
        if declared.contains(register.as_str()) || !reserved.insert(register) {
            return Err(error(
                "machine backend register roles overlap another binding or input",
            ));
        }
    }
    let inout = match backend.integer("add_inout")? {
        0 => false,
        1 => true,
        _ => return Err(error("machine backend add_inout must be 0 or 1")),
    };
    let mut lines = Vec::new();
    let mut clobbers = BTreeSet::new();
    for instruction in instructions {
        let operation = instruction.text("operation")?;
        if !matches!(operation.as_ref(), "copy" | "add" | "load" | "store") {
            return Err(error("unknown generated machine operation"));
        }
        let destination = resolve(&instruction.text("destination")?)?;
        let mut left = resolve(&instruction.text("left")?)?;
        let right = resolve(&instruction.text("right")?)?;
        if operation == "add" {
            if inout && destination != left {
                if instruction.integer("boundary")? != 1 {
                    return Err(error(
                        "this backend's addition requires its destination to be the left operand; copy it explicitly with `::` first",
                    ));
                }
                lines.push(render(&backend.text("copy")?, &destination, &left, &left)?);
                left = destination.clone();
            }
            for value in backend.sequence("add_clobbers")? {
                clobbers.insert(text(value)?.to_string());
            }
        }
        lines.push(render(
            &backend.text(&operation)?,
            &destination,
            &left,
            &right,
        )?);
    }
    for input in &mut spec.inputs {
        input.register = resolve(&input.register)?;
    }
    spec.result = spec.result.as_deref().map(resolve).transpose()?;
    for register in &spec.clobbers {
        clobbers.insert(resolve(register)?);
    }
    spec.clobbers = clobbers.into_iter().collect();
    spec.template = if lines.is_empty() {
        backend.text("nop")?.into()
    } else {
        lines.join("\n")
    };
    Ok(spec)
}

fn render(
    template: &str,
    destination: &str,
    left: &str,
    right: &str,
) -> Result<String, Diagnostic> {
    let rendered = template
        .replace("{destination}", destination)
        .replace("{left}", left)
        .replace("{right}", right);
    if rendered.contains(['{', '}']) {
        return Err(error("unknown machine instruction template placeholder"));
    }
    Ok(rendered)
}

fn text(value: &ConstValue) -> Result<Cow<'_, str>, Diagnostic> {
    value
        .as_utf8()
        .filter(|text| !text.is_empty())
        .ok_or_else(|| error("machine register names must be nonempty compile-time UTF-8 bytes"))
}

fn error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new("type-machine-backend", message.into())
}

fn read_backend<'a>(
    value: &'a ConstValue,
    program: ResolvedProgram<'_>,
    target: &CompileTarget,
) -> Result<Fields<'a>, Diagnostic> {
    let backend = Fields::new(
        value,
        program,
        &[
            "target",
            "arch",
            "dialect",
            "inputs",
            "references",
            "scratch",
            "result",
            "length",
            "copy",
            "add",
            "load",
            "store",
            "nop",
            "add_inout",
            "add_clobbers",
        ],
    )?;
    let CompileTarget::Concrete(target) = target else {
        return Err(error("#machine requires a concrete target"));
    };
    if backend.text("target")? != target.raw || backend.text("arch")? != target.arch.as_str() {
        return Err(error("machine backend does not match the active target"));
    }
    Ok(backend)
}

pub(crate) fn compile_leaf(
    value: &ConstValue,
    program: ResolvedProgram<'_>,
    target: &CompileTarget,
) -> Result<(AssemblySpec, Option<DefId>), Diagnostic> {
    let leaf = Fields::new(value, program, &["machine_leaf", "specification"])?;
    read_backend(leaf.value("machine_leaf"), program, target)?;
    let specification = leaf.value("specification");
    if let Some(source) = specification.as_utf8() {
        return AssemblySpec::parse(&source)
            .map(|spec| (spec, None))
            .map_err(|errors| {
                error(
                    errors
                        .into_iter()
                        .map(|error| error.message)
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            });
    }
    match specification {
        value @ (ConstValue::Record { .. } | ConstValue::Tag { .. }) => {
            let region = EncodingRegion::from_value(value, program)?;
            let evaluation = region.evaluation_method(program)?;
            let spec = region.compile(program, target)?.into_assembly();
            Ok((spec, evaluation))
        }
        _ => Err(error(
            "machine assembly specification must be compile-time text or an instruction region",
        )),
    }
}
