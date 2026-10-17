use ast::{Bind, BindValue, Expr, Typed};
use diagnostic::DiagnosticCode;
use diagnostic::code::ImportCode;

type ArgumentError = (DiagnosticCode, String);

pub(super) fn bind_arguments<'a>(
    template: &Bind,
    arguments: &'a [Typed<Expr>],
) -> Result<Vec<&'a Typed<Expr>>, ArgumentError> {
    let parameters = template
        .params
        .as_ref()
        .expect("import templates have parameters");
    let mut ordered = vec![None; parameters.len()];
    let mut positional = Vec::with_capacity(arguments.len());
    for argument in arguments {
        if let Expr::Bind(bind) = &argument.value {
            let Some(index) = parameters.get_index_of(&bind.name) else {
                return Err((
                    // use-specialization-argument-name
                    DiagnosticCode::Import(ImportCode::SpecializationArgumentName),
                    format!("unknown conditional-import argument `{}`", bind.name),
                ));
            };
            if ordered[index].replace(argument_value(argument)).is_some() {
                return Err((
                    // use-specialization-argument-duplicate
                    DiagnosticCode::Import(ImportCode::SpecializationArgumentDuplicate),
                    format!("duplicate conditional-import argument `{}`", bind.name),
                ));
            }
        } else {
            positional.push(argument);
        }
    }
    let mut positional = positional.into_iter();
    for argument in &mut ordered {
        if argument.is_none() {
            *argument = positional.next();
        }
    }
    if positional.next().is_some() || ordered.iter().any(Option::is_none) {
        return Err((
            // use-specialization-arity
            DiagnosticCode::Import(ImportCode::SpecializationArity),
            format!(
                "conditional-import call requires {} explicit arguments, received {}",
                parameters.len(),
                arguments.len(),
            ),
        ));
    }
    Ok(ordered.into_iter().map(Option::unwrap).collect())
}

pub(super) fn argument_value(argument: &Typed<Expr>) -> &Typed<Expr> {
    if let Expr::Bind(bind) = &argument.value {
        if let BindValue::Expr(value) = &bind.value {
            return value;
        }
    }
    argument
}
