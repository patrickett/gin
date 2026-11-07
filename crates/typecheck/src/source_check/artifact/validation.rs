//! Structural checks at the artifact boundary, not a proof of a body's semantics.
use super::*;

/// A field slot must be paid for by a field record in one of the two layers. In particular,
/// a tiny artifact cannot turn a hidden-field count into an arbitrarily large allocation.
pub(super) fn field_slots(layers: &Layers<'_>) -> Result<usize, ArtifactError> {
    let mut count = 0usize;
    for node in list(&layers.sig[1])?.iter().chain(list(&layers.inst[2])?) {
        let fields = record(node, LAYOUT, 18)?;
        count = count
            .checked_add(list(&fields[3])?.len())
            .ok_or(shape("field budget"))?;
    }
    for node in list(&layers.inst[1])? {
        let fields = record(node, PRIVATE_FIELDS, 2)?;
        count = count
            .checked_add(list(&fields[1])?.len())
            .ok_or(shape("field budget"))?;
    }
    Ok(count)
}

pub(super) fn instances(
    instances: &[CheckedSourceInstance],
    layouts: &[Layout],
) -> Result<(), ArtifactError> {
    for instance in instances {
        let arity = match instance.head {
            CheckedSourceInstanceHead::Nominal { schema } => {
                layouts[schema as usize].schema.parameters.len()
            }
            CheckedSourceInstanceHead::Array { .. }
            | CheckedSourceInstanceHead::Slice
            | CheckedSourceInstanceHead::Pointer => 1,
            CheckedSourceInstanceHead::Sized => 2,
            CheckedSourceInstanceHead::Sequence => continue,
        };
        if instance.arguments.len() != arity {
            return Err(shape("type argument arity"));
        }
    }
    Ok(())
}

pub(super) fn sum_instance(
    id: u32,
    instances: &[CheckedSourceInstance],
    layouts: &[Layout],
) -> Result<(), ArtifactError> {
    match instances[id as usize].head {
        CheckedSourceInstanceHead::Nominal { schema }
            if layouts[schema as usize].schema.kind == AggregateKind::Sum =>
        {
            Ok(())
        }
        _ => Err(shape("summary sum")),
    }
}

pub(super) fn summary(
    summary: &Summary,
    instances: &[CheckedSourceInstance],
    layouts: &[Layout],
) -> Result<(), ArtifactError> {
    let nodes = summary.inputs.len();
    if summary.dispositions.len() != nodes || summary.borrowed.len() != summary.roots.len() {
        return Err(shape("summary shape"));
    }
    let mut bound = vec![false; nodes];
    for root in &summary.roots {
        let slot = bound.get_mut(*root).ok_or(shape("summary root"))?;
        if *slot {
            return Err(shape("duplicate summary root"));
        }
        *slot = true;
    }
    for (index, input) in summary.inputs.iter().enumerate() {
        if !bound[index] {
            return Err(shape("unbound summary input"));
        }
        summary_slots(input.ty, &input.children, &input.slots)?;
        for child in &input.children {
            if *child <= index || *child >= nodes {
                return Err(shape("summary input order"));
            }
            if bound[*child] {
                return Err(shape("duplicate summary input"));
            }
            bound[*child] = true;
        }
    }
    summary_outputs(summary, instances, layouts)?;
    if summary
        .integer_guards
        .iter()
        .any(|(input, _)| *input >= nodes)
        || summary
            .input_ranges
            .iter()
            .any(|(input, low, high)| *input >= nodes || low > high)
    {
        return Err(shape("summary guard"));
    }
    Ok(())
}

fn summary_outputs(
    summary: &Summary,
    instances: &[CheckedSourceInstance],
    layouts: &[Layout],
) -> Result<(), ArtifactError> {
    let mut bound = vec![false; summary.outputs.len()];
    if let Some(root) = bound.first_mut() {
        *root = true;
    }
    for (index, output) in summary.outputs.iter().enumerate() {
        if !bound[index] {
            return Err(shape("unbound summary output"));
        }
        summary_output(output, instances, layouts)?;
        summary_slots(output.ty, &output.children, &output.slots)?;
        if output
            .input
            .is_some_and(|input| input >= summary.inputs.len())
        {
            return Err(shape("summary output origin"));
        }
        for child in &output.children {
            if *child <= index || *child >= bound.len() {
                return Err(shape("summary output order"));
            }
            if bound[*child] {
                return Err(shape("duplicate summary output"));
            }
            bound[*child] = true;
        }
    }
    Ok(())
}

fn summary_output(
    output: &Output,
    instances: &[CheckedSourceInstance],
    layouts: &[Layout],
) -> Result<(), ArtifactError> {
    if output.input.is_some() {
        if !output.children.is_empty() || !output.slots.is_empty() {
            return Err(shape("forwarded summary children"));
        }
        return Ok(());
    }
    if output.materialized {
        for ty in [Some(output.ty), output.structural].into_iter().flatten() {
            if let SourceType::Aggregate { id } = ty {
                let CheckedSourceInstanceHead::Nominal { schema } = instances[id as usize].head
                else {
                    return Err(shape("summary record type"));
                };
                if output.children.len() != layouts[schema as usize].schema.fields.len() {
                    return Err(shape("summary record fields"));
                }
            }
        }
    }
    Ok(())
}

fn summary_slots(ty: SourceType, children: &[usize], slots: &[u32]) -> Result<(), ArtifactError> {
    match ty {
        SourceType::Array { .. } | SourceType::Sequence { .. } => {
            if children.len() != slots.len() || slots.windows(2).any(|pair| pair[0] >= pair[1]) {
                return Err(shape("summary slots"));
            }
        }
        _ => {
            if !slots.is_empty() {
                return Err(shape("summary slots"));
            }
        }
    }
    Ok(())
}

pub(super) fn function(function: &Function) -> Result<(), ArtifactError> {
    let signature = &function.signature;
    let values = function
        .parameters
        .iter()
        .filter(|p| matches!(p, Parameter::Value(_)))
        .count();
    if values != signature.parameters.len()
        || function.implicit_types > signature.type_parameters.len()
    {
        return Err(shape("function parameters"));
    }
    if signature.type_bounds.len() > signature.type_parameters.len() {
        return Err(shape("function type bounds"));
    }
    if signature
        .mutated
        .iter()
        .any(|index| *index as usize >= values)
        || function
            .result_sources
            .iter()
            .any(|source| source.argument >= values)
        || function
            .bound_query
            .is_some_and(|query| query.generic >= signature.type_parameters.len())
    {
        return Err(shape("function parameter index"));
    }
    for summary in &function.summaries {
        if summary.roots.len() != values {
            return Err(shape("summary arity"));
        }
    }
    Ok(())
}

pub(super) fn body_parameters(
    body: &CheckedSourceBody,
    signature: &CheckedSourceFunction,
) -> Result<(), ArtifactError> {
    if body.outputs.len() != signature.mutated.len() {
        return Err(shape("body output arity"));
    }
    for expression in &body.expressions {
        match expression.kind {
            CheckedSourceBodyKind::Parameter { index } => {
                if index as usize >= signature.parameters.len() {
                    return Err(shape("body parameter"));
                }
            }
            CheckedSourceBodyKind::BoundCall { generic, .. }
                if generic as usize >= signature.type_parameters.len() =>
            {
                return Err(shape("body generic"));
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(super) struct FunctionShape {
    values: usize,
    types: usize,
    mutated: usize,
}

impl FunctionShape {
    pub(super) fn of(function: &Function) -> Self {
        Self {
            values: function.signature.parameters.len(),
            types: function.signature.type_parameters.len(),
            mutated: function.signature.mutated.len(),
        }
    }
}

pub(super) fn body_calls(
    body: &CheckedSourceBody,
    callee_shape: impl Fn(CheckedSourceCallee) -> Result<FunctionShape, ArtifactError>,
) -> Result<(), ArtifactError> {
    use CheckedSourceBodyKind as K;
    for expression in &body.expressions {
        match &expression.kind {
            K::Call {
                callee,
                arguments,
                type_arguments,
            } => {
                let shape = callee_shape(*callee)?;
                if arguments.len() != shape.values || type_arguments.len() != shape.types {
                    return Err(super::shape("call arity"));
                }
            }
            K::CallOut { call, index } => {
                let K::Call { callee, .. } = body.expressions[*call as usize].kind else {
                    return Err(shape("call output"));
                };
                if *index as usize >= callee_shape(callee)?.mutated {
                    return Err(shape("call output index"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn body_node(
    kind: &CheckedSourceBodyKind,
    prefix: &[CheckedSourceBodyExpr],
    layouts: &[Layout],
    instances: &[CheckedSourceInstance],
) -> Result<(), ArtifactError> {
    use CheckedSourceBodyKind as K;
    let end = prefix.len() as u32;
    match kind {
        K::Select {
            region: Some(region),
            ..
        } => {
            region_bounds(region.then_start, region.else_start, end)?;
        }
        K::Match { arms, region, .. } => {
            if arms.is_empty() {
                return Err(shape("empty match"));
            }
            if let Some(region) = region {
                region_bounds(region.member_start, region.other_start, end)?;
                if !arms.iter().any(|arm| arm.body == region.member_body) {
                    return Err(shape("match member body"));
                }
            }
        }
        K::Loop {
            start,
            end: limit,
            counter,
            body_start,
            carried,
        } => {
            region_bounds(*body_start, *counter, end)?;
            if *start >= *body_start || *limit >= *body_start {
                return Err(shape("loop bounds"));
            }
            if !matches!(prefix[*counter as usize].kind, K::LoopCounter) {
                return Err(shape("loop counter"));
            }
            loop_carried(carried, *body_start, prefix)?;
        }
        K::While {
            left,
            right,
            body_start,
            split,
            carried,
            exit,
            ..
        } => {
            region_bounds(*body_start, *split, end)?;
            if *left >= *split || *right >= *split {
                return Err(shape("while condition"));
            }
            if exit.is_some_and(|index| index as usize >= carried.len()) {
                return Err(shape("while exit"));
            }
            loop_carried(carried, *body_start, prefix)?;
        }
        K::LoopResult { looped, index } => loop_result(*looped, *index, prefix)?,
        _ => body_projection(kind, prefix, layouts, instances)?,
    }
    Ok(())
}

fn loop_result(
    looped: u32,
    index: u32,
    prefix: &[CheckedSourceBodyExpr],
) -> Result<(), ArtifactError> {
    use CheckedSourceBodyKind as K;
    let carried = match &prefix[looped as usize].kind {
        K::Loop { carried, .. } | K::While { carried, .. } => carried,
        _ => return Err(shape("loop result")),
    };
    if index as usize >= carried.len() {
        return Err(shape("loop result index"));
    }
    Ok(())
}

fn region_bounds(start: u32, split: u32, end: u32) -> Result<(), ArtifactError> {
    if start <= split && split <= end {
        Ok(())
    } else {
        Err(shape("body region"))
    }
}

fn loop_carried(
    carried: &[CheckedSourceLoopCarry],
    start: u32,
    prefix: &[CheckedSourceBodyExpr],
) -> Result<(), ArtifactError> {
    for carry in carried {
        if carry.initial >= start || carry.placeholder < start {
            return Err(shape("loop carry region"));
        }
        let placeholder = prefix
            .get(carry.placeholder as usize)
            .ok_or(shape("loop placeholder"))?;
        if !matches!(placeholder.kind, CheckedSourceBodyKind::LoopCarried) {
            return Err(shape("loop placeholder"));
        }
        if carry.next as usize >= prefix.len() {
            return Err(shape("loop next"));
        }
    }
    Ok(())
}

fn body_projection(
    kind: &CheckedSourceBodyKind,
    prefix: &[CheckedSourceBodyExpr],
    layouts: &[Layout],
    instances: &[CheckedSourceInstance],
) -> Result<(), ArtifactError> {
    use CheckedSourceBodyKind as K;
    match kind {
        K::Construct { schema, fields } => {
            if fields.len() != layouts[*schema as usize].schema.fields.len() {
                return Err(shape("construction arity"));
            }
        }
        K::VariantField { schema, index, .. } => {
            if *index as usize >= layouts[*schema as usize].schema.fields.len() {
                return Err(shape("variant field"));
            }
        }
        K::Field { record, index } => {
            let SourceType::Aggregate { id } = prefix[*record as usize].ty else {
                return Err(shape("field type"));
            };
            let CheckedSourceInstanceHead::Nominal { schema } = instances[id as usize].head else {
                return Err(shape("field schema"));
            };
            if *index as usize >= layouts[schema as usize].schema.fields.len() {
                return Err(shape("field index"));
            }
        }
        K::Part { bundle, index } => {
            let SourceType::Sequence { id } = prefix[*bundle as usize].ty else {
                return Err(shape("bundle type"));
            };
            if *index as usize >= instances[id as usize].arguments.len() {
                return Err(shape("bundle index"));
            }
        }
        K::CallOut { call, .. } if !matches!(prefix[*call as usize].kind, K::Call { .. }) => {
            return Err(shape("call output"));
        }
        _ => {}
    }
    Ok(())
}
