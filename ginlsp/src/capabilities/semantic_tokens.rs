use ginc::lexer::{GinLexer, HasSemanticTokenType};
use ginc::ast::{BindValue, Expr, FileAst, Loop, Return};
use ropey::Rope;
use tower_lsp::lsp_types::{SemanticToken, SemanticTokenType};

pub const LEGEND_TYPE: &[SemanticTokenType] = &[
    SemanticTokenType::FUNCTION,
    SemanticTokenType::STRUCT,
    SemanticTokenType::COMMENT,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::OPERATOR,
    SemanticTokenType::VARIABLE,
    SemanticTokenType::STRING,
    SemanticTokenType::NUMBER,
    SemanticTokenType::PARAMETER,
    SemanticTokenType::CLASS,
    SemanticTokenType::METHOD,
];

pub const TOKEN_FUNCTION: usize = 0;
pub const TOKEN_METHOD: usize = 10;

pub fn parse_tokens(text: &str) -> Vec<(usize, usize, usize)> {
    let mut lex = GinLexer::new(text);
    let mut tokens = Vec::new();

    while let Some((tok, span)) = lex.next_raw() {
        if let Some(token_type) = tok.semantic_token_type_index() {
            tokens.push((span.start, span.end - span.start, token_type));
        }
    }

    tokens
}

fn find_identifier_positions(source: &str, name: &str) -> Vec<(usize, usize)> {
    let mut positions = Vec::new();
    let mut search_start = 0;

    while let Some(pos) = source[search_start..].find(name) {
        let abs_pos = search_start + pos;
        let before_ok = abs_pos == 0
            || !source
                .as_bytes()
                .get(abs_pos - 1)
                .map(|&b| b.is_ascii_alphanumeric() || b == b'_')
                .unwrap_or(false);
        let after_ok = abs_pos + name.len() >= source.len()
            || !source
                .as_bytes()
                .get(abs_pos + name.len())
                .map(|&b| b.is_ascii_alphanumeric() || b == b'_')
                .unwrap_or(false);

        if before_ok && after_ok {
            positions.push((abs_pos, name.len()));
        }
        search_start = abs_pos + name.len();
    }

    positions
}

fn find_method_def_positions(
    source: &str,
    type_name: &str,
    method_name: &str,
) -> Vec<(usize, usize)> {
    let pattern = format!("{}.{}", type_name, method_name);
    let mut positions = Vec::new();
    let mut search_start = 0;

    while let Some(pos) = source[search_start..].find(&pattern) {
        let abs_pos = search_start + pos;
        let before_ok = abs_pos == 0
            || !source
                .as_bytes()
                .get(abs_pos - 1)
                .map(|&b| b.is_ascii_alphanumeric() || b == b'_')
                .unwrap_or(false);
        let after_ok = abs_pos + pattern.len() >= source.len()
            || !source
                .as_bytes()
                .get(abs_pos + pattern.len())
                .map(|&b| b.is_ascii_alphanumeric() || b == b'_')
                .unwrap_or(false);

        if before_ok && after_ok {
            let method_start = abs_pos + type_name.len() + 1;
            positions.push((method_start, method_name.len()));
        }
        search_start = abs_pos + pattern.len();
    }

    positions
}

fn collect_fn_call_names(expr: &Expr, results: &mut Vec<(String, bool)>) {
    match expr {
        Expr::FnCall(call) => {
            let is_method = !call.path.segments.is_empty();
            let name = if is_method {
                call.path.segments.last().unwrap().to_string()
            } else {
                call.path.root.to_string()
            };
            results.push((name, is_method));
            if let Some(args) = &call.args {
                for arg in args {
                    collect_fn_call_names(arg, results);
                }
            }
        }
        Expr::Binary(bin) => {
            collect_fn_call_names(&bin.lhs, results);
            collect_fn_call_names(&bin.rhs, results);
        }
        Expr::Loop(loop_expr) => match loop_expr {
            Loop::ForIn(for_loop) => {
                for e in &for_loop.exprs {
                    collect_fn_call_names(e, results);
                }
            }
            Loop::While(while_loop) => {
                for e in &while_loop.exprs {
                    collect_fn_call_names(e, results);
                }
            }
        },
        Expr::Bind(bind) => {
            collect_fn_call_names_from_bind_value(bind.value(), results);
        }
        Expr::FormatString(fs) => {
            for part in &fs.parts {
                if let ginc::ast::FormatPart::Expr(e) = part {
                    collect_fn_call_names(e, results);
                }
            }
        }
        Expr::Range(range) => {
            collect_fn_call_names(&range.start, results);
            collect_fn_call_names(&range.end, results);
        }
        Expr::Lit(_) => {}
    }
}

fn collect_fn_call_names_from_bind_value(
    bind_value: &BindValue,
    results: &mut Vec<(String, bool)>,
) {
    match bind_value {
        BindValue::Expr(e) => {
            collect_fn_call_names(e, results);
        }
        BindValue::Body { exprs, ret } => {
            for e in exprs {
                collect_fn_call_names(e, results);
            }
            collect_fn_call_names_from_return(ret, results);
        }
    }
}

fn collect_fn_call_names_from_return(ret: &Return, results: &mut Vec<(String, bool)>) {
    if let Some(r) = &ret.0 {
        collect_fn_call_names(r, results);
    }
}

pub fn build_semantic_tokens_from_ast(source: &str, ast: &FileAst) -> Vec<SemanticToken> {
    let mut tokens = parse_tokens(source);

    for bind in ast.defs().values() {
        if bind.is_method() {
            if let Some(type_name) = bind.receiver_type() {
                let method_name = bind.name();
                for (start, len) in
                    find_method_def_positions(source, &type_name.to_string(), method_name.as_str())
                {
                    tokens.push((start, len, TOKEN_METHOD));
                }
            }
        } else {
            let name = bind.name();
            for (start, len) in find_identifier_positions(source, name.as_str()) {
                tokens.push((start, len, TOKEN_FUNCTION));
            }
        }
    }

    let mut call_names: Vec<(String, bool)> = Vec::new();
    for expr in ast.top_level_exprs() {
        collect_fn_call_names(expr, &mut call_names);
    }
    for bind in ast.defs().values() {
        collect_fn_call_names_from_bind_value(bind.value(), &mut call_names);
    }

    for (name, is_method) in call_names {
        let token_type = if is_method {
            TOKEN_METHOD
        } else {
            TOKEN_FUNCTION
        };
        for (start, len) in find_identifier_positions(source, &name) {
            tokens.push((start, len, token_type));
        }
    }

    tokens.sort_by_key(|a| a.0);
    tokens.dedup_by_key(|a| a.0);

    let rope = Rope::from_str(source);
    let mut pre_line: u32 = 0;
    let mut pre_start: u32 = 0;

    tokens
        .iter()
        .filter_map(|(start, length, token_type)| {
            let line = rope.try_byte_to_line(*start).ok()? as u32;
            let first_char_of_line = rope.try_line_to_char(line as usize).ok()? as u32;
            let start_in_line = rope.try_byte_to_char(*start).ok()? as u32 - first_char_of_line;
            let delta_line = line - pre_line;
            let delta_start = if delta_line == 0 {
                start_in_line - pre_start
            } else {
                start_in_line
            };

            let ret = SemanticToken {
                delta_line,
                delta_start,
                length: *length as u32,
                token_type: *token_type as u32,
                token_modifiers_bitset: 0,
            };
            pre_line = line;
            pre_start = start_in_line;

            Some(ret)
        })
        .collect()
}
