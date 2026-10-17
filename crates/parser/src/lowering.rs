use std::collections::HashMap;

use ast::FileAst;
use internment::Intern;

pub(crate) fn lower(file_ast: &mut FileAst) {
    let mut members = HashMap::new();
    for bind in &file_ast.method_binds {
        let Some((receiver, method)) = method_base_name(bind) else {
            continue;
        };
        let name = Intern::new(format!("{receiver}.{method}"));
        let member = members.entry(name).or_insert(0);
        let id = ast::DefinitionId::member(name, *member);
        *member += 1;
        file_ast.defs.insert(id, bind.clone());
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

    // `parse_expression` cannot be passed as a function item: its `&mut self`
    // receiver does not unify with the higher-ranked `ExprFn` pointer lifetime.
    #[allow(clippy::redundant_closure)]
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
        parser.parse_file_without_lowering(|cursor| TokenCursor::parse_expression(cursor))
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
