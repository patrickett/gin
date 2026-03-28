use crate::util::{format_params, is_identifier_char};
use ginc::{ast::Tag, prelude::SimpleSpan, FileAst};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, Documentation, MarkupContent, MarkupKind, Position, Url,
};

pub fn use_completions(
    source: &str,
    position: Position,
    file_uri: &Url,
    config: Option<&flask::FlaskConfigHandle>,
) -> Option<Vec<CompletionItem>> {
    let line_text = source.lines().nth(position.line as usize)?;
    let trimmed = line_text.trim_start();

    if !trimmed.starts_with("use ") {
        return None;
    }

    let col = position.character as usize;
    let before_cursor = &line_text[..col.min(line_text.len())];

    if let Some(quote_pos) = before_cursor.rfind('\'') {
        let partial = &before_cursor[quote_pos + 1..];
        return Some(complete_local_paths(file_uri, partial));
    }

    Some(complete_dependency_names(file_uri, config))
}

fn complete_local_paths(file_uri: &Url, partial: &str) -> Vec<CompletionItem> {
    let file_path = match file_uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_dir = match file_path.parent() {
        Some(d) => d,
        None => return vec![],
    };

    let search_dir = if partial.is_empty() {
        file_dir.to_path_buf()
    } else {
        let partial_path = std::path::Path::new(partial);
        let resolved = file_dir.join(partial_path);
        if partial.ends_with('/') {
            resolved
        } else {
            resolved.parent().unwrap_or(&resolved).to_path_buf()
        }
    };

    let prefix = if partial.contains('/') && !partial.ends_with('/') {
        let last_slash = partial.rfind('/').unwrap();
        &partial[..=last_slash]
    } else if partial.ends_with('/') {
        partial
    } else {
        ""
    };

    let mut items = Vec::new();
    let entries = match std::fs::read_dir(&search_dir) {
        Ok(e) => e,
        Err(_) => return items,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let path = entry.path();
        let is_dir = path.is_dir();
        let is_gin = path.extension().is_some_and(|e| e == "gin");

        if !is_dir && !is_gin {
            continue;
        }

        let label = if is_dir {
            format!("{prefix}{name}/")
        } else {
            let stem = path.file_stem().unwrap_or_default().to_string_lossy();
            format!("{prefix}{stem}")
        };

        let kind = if is_dir {
            CompletionItemKind::FOLDER
        } else {
            CompletionItemKind::FILE
        };

        items.push(CompletionItem {
            label,
            kind: Some(kind),
            detail: Some(if is_dir {
                "directory".to_string()
            } else {
                "gin module".to_string()
            }),
            ..Default::default()
        });
    }

    items
}

fn complete_dependency_names(
    file_uri: &Url,
    config: Option<&flask::FlaskConfigHandle>,
) -> Vec<CompletionItem> {
    // Use cached config if available
    if let Some(handle) = config {
        let cfg = handle.read();
        return cfg
            .dependency_names()
            .into_iter()
            .map(|name| CompletionItem {
                label: name.to_string(),
                kind: Some(CompletionItemKind::MODULE),
                detail: Some("dependency".to_string()),
                ..Default::default()
            })
            .collect();
    }

    // Fallback: load fresh (for when config wasn't cached yet)
    let file_path = match file_uri.to_file_path() {
        Ok(p) => p,
        Err(_) => return vec![],
    };
    let file_dir = match file_path.parent() {
        Some(d) => d,
        None => return vec![],
    };

    let config = match flask::FlaskConfig::from_directory(file_dir) {
        Some(c) => c,
        None => return vec![],
    };

    config
        .dependency_names()
        .into_iter()
        .map(|name| CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::MODULE),
            detail: Some("dependency".to_string()),
            ..Default::default()
        })
        .collect()
}

pub fn build_completions(ast: &FileAst) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = Vec::new();

    for (name, decl) in ast.tags() {
        let detail = decl
            .params()
            .as_ref()
            .map(|p| format!("tag {}{}", name, format_params(p)));
        let documentation = decl.doc_comment().map(|dc| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: dc.0.clone(),
            })
        });
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(CompletionItemKind::CLASS),
            detail,
            documentation,
            ..Default::default()
        });
    }

    for (name, bind) in ast.defs() {
        let is_fn = bind.params().is_some();
        let kind = if is_fn {
            CompletionItemKind::FUNCTION
        } else {
            CompletionItemKind::VARIABLE
        };
        let detail = bind
            .params()
            .as_ref()
            .map(|p| format!("{}{}", name.as_str(), format_params(p)));
        let documentation = bind.doc_comment().map(|dc| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: dc.0.clone(),
            })
        });
        items.push(CompletionItem {
            label: name.as_str().to_string(),
            kind: Some(kind),
            detail,
            documentation,
            ..Default::default()
        });
    }

    let keywords = ["if", "else", "for", "in", "while", "return", "use", "tag"];
    for kw in keywords {
        items.push(CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        });
    }

    items
}

pub fn extract_fn_name_before_paren(line_text: &str) -> Option<String> {
    if let Some(paren_pos) = line_text.rfind('(') {
        let before_paren = line_text[..paren_pos].trim_end();
        let fn_name: String = before_paren
            .chars()
            .rev()
            .take_while(|c| is_identifier_char(*c))
            .collect::<String>()
            .chars()
            .rev()
            .collect();

        if !fn_name.is_empty() {
            return Some(fn_name);
        }
    }
    None
}

/// Provide completions after a dot (.) for union variants and methods.
///
/// This is triggered when the cursor is after a dot, like `val.|` or `self.|`.
/// It analyzes the expression before the dot and provides context-aware completions:
/// - For union types: shows all variants (e.g., `Some`, `None` for `Maybe`)
/// - For method calls: could show available methods on the type (TODO)
pub fn dot_completions(
    source: &str,
    position: Position,
    ast: &FileAst,
    ty_env: &ginc::typeck::TyEnv,
) -> Option<Vec<CompletionItem>> {
    use ginc::intern::IStr;

    // Get the line up to the cursor position
    let line = source.lines().nth(position.line as usize)?;
    let byte_pos = position.character as usize;
    let before_cursor = if byte_pos <= line.len() {
        &line[..byte_pos]
    } else {
        line
    };

    // Find the expression before the last dot
    // We need to find the last dot and then parse what comes before it
    let dot_pos = before_cursor.rfind('.')?;
    let before_dot = &before_cursor[..dot_pos];

    // Extract the identifier/expression before the dot
    let identifier: String = before_dot
        .chars()
        .rev()
        .take_while(|c| is_identifier_char(*c))
        .collect::<String>()
        .chars()
        .rev()
        .collect();

    if identifier.is_empty() {
        return None;
    }

    let ident_str = IStr::new(identifier.clone());

    // First, try to resolve as a direct type tag (e.g., `Maybe.`, `Bool.`)
    let initial_ty = ty_env.resolve_tag(&Tag::Nominal(ident_str, SimpleSpan::from(0..0)));

    // If the type is the default (Int(64) for unknown), check if it's a known tag first
    // (e.g., Bool is a union declaration, not a variable binding)
    let ty = match initial_ty {
        ginc::typeck::Ty::Int(64) => {
            // Check if this is actually a known tag/union declaration
            if ast.tags().contains_key(&ident_str) {
                // This is a valid tag - build the Union type from the declaration
                if let Some(decl) = ast.tags().get(&ident_str) {
                    if let ginc::ast::DeclareValue::Union { variants } = decl.value() {
                        // Build Ty::Union from the variants
                        let mut variant_vec = Vec::new();
                        for variant in variants {
                            let tag = variant.tag();
                            if let ginc::ast::Tag::Generic(_, params, _) = tag {
                                let mut fields = Vec::new();
                                for (name, kind) in params {
                                    use ginc::ast::ParameterKind;
                                    let ty = match kind {
                                        ParameterKind::Generic => {
                                            ginc::typeck::Ty::Opaque(IStr::new(name.to_string()))
                                        }
                                        ParameterKind::Tagged(inner_tag) => {
                                            // For nested tags, just store as opaque for now
                                            ginc::typeck::Ty::Opaque(IStr::new(
                                                inner_tag.to_string(),
                                            ))
                                        }
                                        ParameterKind::Default(_) => ginc::typeck::Ty::Int(64),
                                    };
                                    fields.push((*name, Box::new(ty)));
                                }
                                variant_vec.push((IStr::new(tag.name().to_string()), fields));
                            } else {
                                // Nominal variant with no fields
                                variant_vec.push((IStr::new(tag.name().to_string()), Vec::new()));
                            }
                        }
                        ginc::typeck::Ty::Union {
                            name: ident_str,
                            variants: variant_vec,
                        }
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            } else {
                // Not a tag, look for a variable binding with that name
                use ginc::ast::BindValue;

                fn find_bind_in_exprs<'a>(
                    exprs: &'a [ginc::ast::Spanned<ginc::ast::Expr>],
                    name: &ginc::intern::IStr,
                ) -> Option<&'a ginc::ast::Bind> {
                    for expr in exprs {
                        if let ginc::ast::Expr::Bind(bind) = &**expr {
                            if bind.name() == *name {
                                return Some(bind);
                            }
                        }
                    }
                    None
                }

                // First, look at top-level defs
                let found_bind =
                    ast.defs()
                        .values()
                        .find(|b| b.name() == ident_str)
                        .or_else(|| {
                            // If not found at top level, search in function bodies
                            ast.defs().values().find_map(|bind| {
                                if let BindValue::Body { exprs, .. } = bind.value() {
                                    find_bind_in_exprs(exprs, &ident_str)
                                } else {
                                    None
                                }
                            })
                        });

                if let Some(bind) = found_bind {
                    // Get the type annotation from the binding
                    if let Some((type_name, _)) = &bind.type_annotation {
                        let type_tag = Tag::Nominal(*type_name, SimpleSpan::from(0..1));
                        ty_env.resolve_tag(&type_tag)
                    } else {
                        return None;
                    }
                } else {
                    return None;
                }
            }
        }
        _ => initial_ty,
    };

    // Check if this is a union type and provide variant completions
    if let ginc::typeck::Ty::Union { variants, .. } = ty {
        let mut items = Vec::new();

        for (variant_name, fields) in variants {
            // Build label showing just variant and params (e.g., "Some(x)")
            let label = if fields.is_empty() {
                variant_name.to_string()
            } else {
                let field_names: Vec<String> =
                    fields.iter().map(|(name, _)| name.to_string()).collect();
                format!("{}({})", variant_name, field_names.join(", "))
            };

            // Build insert_text same as label (just the variant part)
            let insert_text = label.clone();

            // Detail shows the full qualified name for context
            let qualified_name = format!("{}.{}", identifier, variant_name);
            let detail = if fields.is_empty() {
                qualified_name
            } else {
                let field_names: Vec<String> =
                    fields.iter().map(|(name, _)| name.to_string()).collect();
                format!("{}({})", qualified_name, field_names.join(", "))
            };

            items.push(CompletionItem {
                label,
                insert_text: Some(insert_text),
                kind: Some(CompletionItemKind::ENUM_MEMBER),
                detail: Some(detail),
                ..Default::default()
            });
        }

        if !items.is_empty() {
            return Some(items);
        }
    }

    // TODO: Add method completions for record types and built-in types

    None
}
