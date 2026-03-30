mod expr;
pub use expr::*;
#[allow(clippy::module_inception)]
mod ast;
pub use ast::*;
mod parameter;
pub use parameter::*;
mod path;
pub use path::*;
mod tag;
pub use tag::*;
mod doc_comment;
pub use doc_comment::*;
mod declare;
pub use declare::*;
mod pattern;
pub use pattern::*;
mod ident;
pub use ident::*;
mod spanned;
pub use spanned::*;
mod impl_block;
pub use impl_block::*;

pub use expr::FormatPart;
use crate::prelude::*;
use chumsky::{input::ValueInput, span::SimpleSpan};
use std::collections::HashSet;

/// Parses a stream of tokens into a `FileAst`.
pub fn token_parser<'t, I>() -> impl Parser<'t, I, FileAst, crate::parse::ParserError<'t>>
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    use Token::*;

    let expr_parser = expression();

    let method_parser = tag(expr_parser.clone())
        .then_ignore(just(Dot))
        .then(bind(expr_parser.clone()))
        .map(|(receiver_type, bind)| bind.with_receiver_type(Some(receiver_type)));

    let element = choice((
        impl_block(expr_parser.clone()).map(TopLevelValue::ImplBlock),
        method_parser.map(TopLevelValue::Bind),
        bind(expr_parser.clone()).map(TopLevelValue::Bind),
        declare(expr_parser.clone()).map(TopLevelValue::Tag),
        expr_parser.map(|Spanned(expr, span)| TopLevelValue::Expr(expr, span)),
    ))
    .padded_by(just(Newline).repeated());

    import()
        .separated_by(just(Newline))
        .collect::<Vec<_>>()
        .then(element.clone().repeated().collect::<Vec<_>>())
        .then(
            just(Private)
                .padded_by(just(Newline).repeated())
                .ignore_then(element.repeated().collect::<Vec<_>>())
                .or_not(),
        )
        .map(|((imports, public_elements), private_elements)| {
            let mut tags = TagMap::new();
            let mut defs = DefMap::new();
            let mut private_defs = std::collections::HashSet::new();
            let mut private_tags = std::collections::HashSet::new();
            let mut exprs = Vec::new();

            for el in public_elements {
                collect_top_level(el, &mut tags, &mut defs, &mut exprs);
            }

            if let Some(priv_elements) = private_elements {
                for el in priv_elements {
                    match &el {
                        TopLevelValue::Tag(decl) => {
                            private_tags.insert(decl.name());
                        }
                        TopLevelValue::Bind(bind) => {
                            private_defs.insert(bind.name());
                        }
                        TopLevelValue::ImplBlock(block) => {
                            for method_name in block.methods.keys() {
                                let mangled = IStr::new(format!(
                                    "{}.{}",
                                    block.type_name.as_str(),
                                    method_name.as_str()
                                ));
                                private_defs.insert(mangled);
                            }
                        }
                        TopLevelValue::Expr(..) => {}
                    }
                    collect_top_level(el, &mut tags, &mut defs, &mut exprs);
                }
            }

            // Generate return type unions from bind return values
            generate_return_type_unions(&defs, &mut tags, &private_defs);

            FileAst {
                uses: imports,
                tags,
                defs,
                private_defs,
                private_tags,
                exprs,
            }
        })
}

enum TopLevelValue {
    Tag(Declare),
    Bind(Bind),
    ImplBlock(ImplBlock),
    Expr(Expr, SimpleSpan),
}

fn collect_top_level(
    el: TopLevelValue,
    tags: &mut TagMap,
    defs: &mut DefMap,
    exprs: &mut Vec<(Expr, SimpleSpan)>,
) {
    match el {
        TopLevelValue::Tag(decl) => {
            let name = decl.name();
            tags.insert(name, decl);
        }
        TopLevelValue::Bind(bind) => {
            let name = if let Some(recv) = bind.receiver_type() {
                IStr::new(format!("{}.{}", recv.name(), bind.name()))
            } else {
                bind.name()
            };
            defs.insert(name, bind);
        }
        TopLevelValue::ImplBlock(block) => {
            let recv_tag = Tag::Nominal(block.type_name, block.type_name_span);
            for (method_name, bind) in block.methods {
                let bind = bind.with_receiver_type(Some(recv_tag.clone()));
                let mangled = IStr::new(format!(
                    "{}.{}",
                    block.type_name.as_str(),
                    method_name.as_str()
                ));
                defs.insert(mangled, bind);
            }
        }
        TopLevelValue::Expr(expr, span) => {
            exprs.push((expr, span));
        }
    }
}

/// Generate return type unions from anonymous return values in binds.
///
/// For example, if a bind `print` has `return PrintSuccess`, this generates
/// a union type. If the bind has a named return type (e.g., `print() result:`),
/// the union is inserted into the tags map with that name. Otherwise, the union
/// is conceptually the return type but not externally referenceable.
///
/// Example with named return type:
///   print(arg) print_result:
///       if arg = 1 return PrintFail
///   return PrintSuccess
///   -> Creates a union `print_result` with variants [PrintFail, PrintSuccess]
///
/// Example with anonymous return type:
///   print(arg):
///       if arg = 1 return PrintFail
///   return PrintSuccess
///   -> Return type is PrintFail or PrintSuccess (not externally referenceable)
fn generate_return_type_unions(
    defs: &DefMap,
    tags: &mut TagMap,
    _private_defs: &std::collections::HashSet<IStr>,
) {
    for bind in defs.values() {
        // Extract anonymous tag names from the bind's return value
        let tag_names = extract_anonymous_tags_from_bind(bind);

        if tag_names.is_empty() {
            continue;
        }

        // Deduplicate tag names (keep one span per name)
        let unique_tags: HashSet<_> = tag_names.into_iter().collect();

        let variants: Vec<Variant> = unique_tags
            .into_iter()
            .map(|(name, span)| Variant::External(Tag::Nominal(name, span)))
            .collect();

        // Only create a named union declaration if return_type_name is provided
        if let Some(name) = bind.return_type_name() {
            // Named return type - the union can be referenced elsewhere
            let decl = Declare::new(
                *name,
                SimpleSpan::from(0..0),
                DeclareValue::Union { variants },
            );
            tags.insert(decl.name(), decl);
        }
        // If no name provided, the union IS the return type but has no external declaration
        // The return type is implicitly: variant1 or variant2 or variant3...
    }
}

/// Extract all anonymous tag names from a bind's return value.
fn extract_anonymous_tags_from_bind(bind: &crate::ast::expr::Bind) -> Vec<(IStr, SimpleSpan)> {
    use crate::ast::expr::BindValue;

    let mut tags = Vec::new();

    match bind.value() {
        BindValue::Expr(expr) => {
            extract_anonymous_tags_from_expr(expr, &mut tags);
        }
        BindValue::Body { exprs, ret } => {
            for expr in exprs {
                extract_anonymous_tags_from_expr(expr, &mut tags);
            }
            if let Some(expr) = &ret.0 {
                extract_anonymous_tags_from_expr(expr, &mut tags);
            }
        }
        BindValue::Extern => {}
    }

    tags
}

/// Recursively extract anonymous tag names from an expression.
fn extract_anonymous_tags_from_expr(
    expr: &crate::ast::expr::Expr,
    tags: &mut Vec<(IStr, SimpleSpan)>,
) {
    use crate::ast::expr::Expr::*;

    match expr {
        AnonymousTag(name, span) => {
            tags.push((*name, *span));
        }
        FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    extract_anonymous_tags_from_expr(arg, tags);
                }
            }
        }
        Binary(bin) => {
            extract_anonymous_tags_from_expr(&bin.lhs, tags);
            extract_anonymous_tags_from_expr(&bin.rhs, tags);
        }
        Loop(loop_expr) => {
            use crate::ast::expr::Loop;
            match loop_expr {
                Loop::ForIn(for_loop) => {
                    for expr in &for_loop.exprs {
                        extract_anonymous_tags_from_expr(expr, tags);
                    }
                    extract_anonymous_tags_from_expr(&for_loop.iter, tags);
                }
                Loop::While(while_loop) => {
                    for expr in &while_loop.exprs {
                        extract_anonymous_tags_from_expr(expr, tags);
                    }
                    extract_anonymous_tags_from_expr(&while_loop.cond, tags);
                }
            }
        }
        When(when_expr) => {
            if let Some(subject) = &when_expr.subject {
                extract_anonymous_tags_from_expr(subject, tags);
            }
            for arm in &when_expr.arms {
                use crate::ast::expr::when::WhenArm;
                match arm {
                    WhenArm::Cond { condition, body } => {
                        extract_anonymous_tags_from_expr(condition, tags);
                        extract_anonymous_tags_from_expr(body, tags);
                    }
                    WhenArm::Is { body, .. } => {
                        extract_anonymous_tags_from_expr(body, tags);
                    }
                    WhenArm::Else(body) => {
                        extract_anonymous_tags_from_expr(body, tags);
                    }
                }
            }
        }
        Bind(bind) => {
            // Local binds can also contain anonymous tags
            use crate::ast::expr::BindValue;
            match bind.value() {
                BindValue::Expr(e) => {
                    extract_anonymous_tags_from_expr(e, tags);
                }
                BindValue::Body { exprs, ret } => {
                    for expr in exprs {
                        extract_anonymous_tags_from_expr(expr, tags);
                    }
                    if let Some(expr) = &ret.0 {
                        extract_anonymous_tags_from_expr(expr, tags);
                    }
                }
                BindValue::Extern => {}
            }
        }
        _ => {}
    }
}
