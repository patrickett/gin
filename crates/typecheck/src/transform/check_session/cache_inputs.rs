use ast::{Bind, FileAst, Parameter, ParameterKind, Parameters};

use super::TransformCtx;

pub(super) fn same_semantic_context(left: &TransformCtx, right: &TransformCtx) -> bool {
    macro_rules! fields {
        ($($field:ident),+ $(,)?) => {{
            let TransformCtx { compile_time_eval_ast: _, $($field),+ } = left;
            true $(&& $field == &right.$field)+
        }};
    }
    fields!(
        package_instance,
        type_registry,
        cross_file_tag_types,
        cross_file_tag_params,
        cross_file_fn_return_types,
        cross_file_callable_signatures,
        cross_file_terminal_exits,
        overload_groups,
        cross_file_variant_map,
        ide_package,
        compile_target,
    )
}

pub(super) fn same_package_declarations(left: &FileAst, right: &FileAst) -> bool {
    macro_rules! fields {
        ($($field:ident),+ $(,)?) => {{
            let FileAst { defs, $($field),+ } = left;
            true $(&& $field == &right.$field)+
                && defs.keys().eq(right.defs.keys())
                && defs.groups() == right.defs.groups()
                && defs.iter().all(|(id, bind)| same_bind_declaration(bind, &right.defs[id]))
        }};
    }
    fields!(
        semantic_origin,
        module_doc,
        uses,
        tags,
        method_binds,
        private_defs,
        private_tags,
        exprs,
        symbol_aliases,
        symbol_alias_spans,
        span_table,
        line_breaks,
        blank_lines,
        parse_warnings,
        todo_holes,
    )
}

fn same_bind_declaration(left: &Bind, right: &Bind) -> bool {
    macro_rules! fields {
        ($($field:ident),+ $(,)?) => {{
            let Bind { value, name_span: _, params, receiver_type, return_tag, $($field),+ } = left;
            std::mem::discriminant(value) == std::mem::discriminant(&right.value)
                && same_parameters(params, &right.params)
                && receiver_type.as_ref().map(|surface| &surface.value)
                    == right.receiver_type.as_ref().map(|surface| &surface.value)
                && return_tag.as_ref().map(|surface| &surface.value)
                    == right.return_tag.as_ref().map(|surface| &surface.value)
                $(&& $field == &right.$field)+
        }};
    }
    fields!(
        doc_comment,
        name,
        param_slots,
        param_conventions,
        group_params,
        param_groups,
        param_refinements,
        attributes,
        receiver_typevars,
        return_type_name,
        named_results,
        return_refinement,
        anonymous_result_alternatives,
        payload_postconditions,
        operator,
        return_type,
        type_annotation,
        type_annotation_qual,
    )
}

fn same_parameters(left: &Option<Parameters>, right: &Option<Parameters>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|((name, left), (other, right))| {
                        name == other && same_parameter(left, right)
                    })
        }
        _ => false,
    }
}

fn same_parameter(left: &Parameter, right: &Parameter) -> bool {
    let Parameter {
        kind,
        name_span: _,
        constraints,
        range_pattern,
    } = left;
    let same_kind = match (kind, &right.kind) {
        (ParameterKind::Tagged(left), ParameterKind::Tagged(right)) => left.value == right.value,
        _ => kind == &right.kind,
    };
    same_kind && constraints == &right.constraints && range_pattern == &right.range_pattern
}
