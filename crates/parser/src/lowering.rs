use std::collections::HashMap;

use ast::FileAst;
use internment::Intern;

pub(crate) fn lower(file_ast: &mut FileAst) {
    canonicalize_method_bind_names(file_ast);
}

fn canonicalize_method_bind_names(file_ast: &mut FileAst) {
    let mut overload_counts = HashMap::<(Intern<String>, Intern<String>), usize>::new();

    for bind in &file_ast.method_binds {
        let Some(base_name) = method_base_name(bind) else {
            continue;
        };

        *overload_counts.entry(base_name).or_default() += 1;
    }

    if overload_counts.is_empty() {
        return;
    }

    let mut canonical_defs = file_ast.defs.clone();

    for method_bind in &file_ast.method_binds {
        let Some(base_name) = method_base_name(method_bind) else {
            continue;
        };
        let key = canonical_method_key(method_bind, base_name, &overload_counts);
        canonical_defs.insert(key, method_bind.clone());
    }

    file_ast.defs = canonical_defs;
}

fn canonical_method_key(
    bind: &ast::Bind,
    (receiver_name, method_name): (Intern<String>, Intern<String>),
    overload_counts: &HashMap<(Intern<String>, Intern<String>), usize>,
) -> Intern<String> {
    let base_key = format!("{receiver_name}.{method_name}");
    if overload_counts[&(receiver_name, method_name)] > 1 {
        let arity = bind.params.as_ref().map_or(0, |params| params.len());
        Intern::new(format!("{base_key}$arity{arity}"))
    } else {
        Intern::new(base_key)
    }
}

fn method_base_name(bind: &ast::Bind) -> Option<(Intern<String>, Intern<String>)> {
    let receiver = bind.receiver_type_surface()?;

    let receiver_name = match &receiver.value {
        ast::Expr::AnonymousTag(name) => *name,
        ast::Expr::TagCall(call) => call.name,
        ast::Expr::FnCall(call) => call
            .path
            .value
            .segments
            .last()
            .copied()
            .unwrap_or(call.path.value.root),
        _ => return None,
    };

    Some((receiver_name, bind.name))
}

#[cfg(test)]
mod tests {
    use super::lower;
    use crate::cursor::TokenCursor;
    use lexer::Lexer;

    fn parse_file_without_lowering(source: &str) -> ast::FileAst {
        let mut lexer = Lexer::new(source);
        let tokens: Vec<_> = lexer.by_ref().collect();
        let mut span_table = lexer.take_span_table();

        let filtered_tokens: Vec<_> = tokens
            .iter()
            .filter(|(t, _)| !matches!(t, lexer::Token::Comment(_)))
            .copied()
            .collect();

        let mut parser = TokenCursor::new(&filtered_tokens, &mut span_table);
        parser.parse_file_for_test(|cursor| TokenCursor::parse_expression(cursor))
    }

    #[test]
    fn method_canonicalization_keeps_non_method_defs_and_warnings() {
        let source = "counter: 1\nanswer := 2\n";
        let mut ast = parse_file_without_lowering(source);
        let baseline = ast.clone();

        lower(&mut ast);

        assert_eq!(ast.defs, baseline.defs);
        assert_eq!(ast.parse_warnings, baseline.parse_warnings);
    }
}
