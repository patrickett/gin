use crate::ast::tag::Variant;
use crate::prelude::*;
use chumsky::span::SimpleSpan;
use std::hash::{Hash, Hasher};

mod attributes;
mod value;
pub use attributes::*;
pub use value::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declare {
    doc_comment: Option<DocComment>,
    attributes: DeclareAttributes,
    name: IStr,
    pub name_span: SimpleSpan,
    params: Option<Parameters>,
    value: DeclareValue,
}

impl Declare {
    pub fn new(name: IStr, name_span: SimpleSpan, value: DeclareValue) -> Self {
        Declare {
            doc_comment: None,
            attributes: DeclareAttributes::default(),
            name,
            name_span,
            params: None,
            value,
        }
    }

    pub fn with_params(mut self, params: Option<Parameters>) -> Self {
        self.params = params;
        self
    }

    pub fn with_doc(mut self, doc: Option<DocComment>) -> Self {
        self.doc_comment = doc;
        self
    }

    pub fn doc_comment(&self) -> Option<&DocComment> {
        self.doc_comment.as_ref()
    }

    pub fn name(&self) -> IStr {
        self.name
    }

    pub fn params(&self) -> &Option<Parameters> {
        &self.params
    }

    pub fn value(&self) -> &DeclareValue {
        &self.value
    }
}

impl std::fmt::Display for Declare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name.as_str())?;
        if let Some(params) = &self.params {
            write!(f, "(")?;
            let mut first = true;
            for (k, v) in params {
                if !first {
                    write!(f, ", ")?;
                }
                first = false;
                write!(f, "{}{v}", k.as_str())?;
            }
            write!(f, ")")?;
        }
        let keyword = match &self.value {
            DeclareValue::Record(_) => " has",
            _ => " is",
        };
        write!(f, "{keyword} {}", self.value)
    }
}

impl Hash for Declare {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
        self.name_span.start.hash(state);
        self.name_span.end.hash(state);
        match &self.params {
            None => 0u8.hash(state),
            Some(params) => {
                1u8.hash(state);
                for (k, v) in params {
                    k.hash(state);
                    v.hash(state);
                }
            }
        }
        self.value.hash(state);
    }
}

pub fn declare<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Declare, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let params = params(expr.clone(), tag(expr.clone()));
    let tag_name = select! { Token::Tag(name) => IStr::new(name.to_string()) }
        .map_with(|name, e| (name, e.span()));

    let lhs_has = tag_name.clone()
        .then(params.clone().or_not())
        .then_ignore(just(Token::Has))
        .then_ignore(just(Token::Newline).or_not()) // Consume optional newline after Has
        .then_ignore(doc_comment().or_not()) // Consume optional doc comment after Has
        .then_ignore(just(Token::Newline).repeated())
        .then_ignore(just(Token::Indent).or_not());

    let lhs_is = tag_name
        .then(params.clone().or_not())
        .then_ignore(just(Token::Is))
        .then(doc_comment().or_not())
        .then_ignore(just(Token::Newline).or_not())
        .then_ignore(just(Token::Newline).repeated())
        .then_ignore(just(Token::Indent).or_not());

    let rhs_record = choice((
        tag(expr.clone()).map(DeclareValue::Alias),
        params.map(DeclareValue::Record),
    ));

    // Parse a variant with optional doc comment BEFORE or AFTER the tag
    // Syntax: [--- doc] Tag [--- doc]
    // Doc after tag takes precedence over doc before tag
    let parse_variant = doc_comment()
        .or_not()
        .then_ignore(just(Token::Newline).or_not())
        .then(tag(expr.clone()))
        .then(doc_comment().or_not()) // Doc after tag
        .then_ignore(just(Token::Newline).or_not())
        .map(|((doc_before, tag), doc_after)| {
            // Prefer doc_after over doc_before
            let doc = doc_after.or(doc_before).filter(|d| !d.0.is_empty());
            if let Some(doc) = doc {
                Variant::Local {
                    doc_comment: Some(doc),
                    tag,
                }
            } else {
                Variant::External(tag)
            }
        });

    // Parse: or [--- doc_on_same_line] variant
    // Doc comment on SAME LINE as `or` belongs to PREVIOUS variant
    // Doc comment on SEPARATE LINE belongs to NEXT variant (handled by parse_variant)
    let parse_or_and_variant = just(Token::Indent)
        .or_not()
        .ignore_then(just(Token::Or))
        .then(doc_comment().or_not()) // Doc on same line (no newline consumed yet)
        .then_ignore(just(Token::Newline).or_not())
        .then_ignore(just(Token::Indent).or_not())
        .then(parse_variant.clone())
        .map(|((_, doc_on_same_line), variant)| (doc_on_same_line, variant));

    // Parse union declarations: A or B or C
    // Doc comments on same line after `or` belong to the PREVIOUS variant
    // Doc comments on separate line (before tag) belong to that tag
    // Syntax: A or --- doc for A
    //         --- doc for B
    //         B
    let rhs_union = parse_variant
        .clone()
        .then(
            parse_or_and_variant
                .repeated()
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .map(|(first, rest)| {
            let mut variants = Vec::with_capacity(rest.len() + 1);
            variants.push(first);
            for (doc_on_same_line, variant) in rest {
                // Associate doc on same line as `or` with the PREVIOUS variant
                if let Some(doc) = doc_on_same_line.filter(|d| !d.is_empty())
                    && let Some(prev) = variants.last_mut()
                {
                    // Only add doc to previous if it doesn't already have one
                    match prev {
                        Variant::External(tag) => {
                            let tag = tag.clone();
                            *prev = Variant::Local {
                                doc_comment: Some(doc),
                                tag,
                            };
                        }
                        Variant::Local { doc_comment, .. } => {
                            if doc_comment.is_none() {
                                *doc_comment = Some(doc);
                            }
                        }
                    }
                }
                variants.push(variant);
            }
            DeclareValue::Union { variants }
        });

    let rhs_union_or_range = choice((
        just(Token::In)
            .ignore_then(int_range())
            .map(DeclareValue::InRange),
        int_range().map(DeclareValue::Range),
        rhs_union,
        tag(expr.clone()).map(DeclareValue::Alias),
    ))
    .then(doc_comment().or_not())
    .then_ignore(just(Token::Dedent).or_not());

    let decl_has = lhs_has
        .then(rhs_record)
        .map(|((( name, name_span), params), value)| {
            Declare::new(name, name_span, value).with_params(params)
        });

    let decl_is = lhs_is.then(rhs_union_or_range).map(
        |((((name, name_span), params), doc_after_is), (value, doc_after_value))| {
            let doc = doc_after_value
                .or(doc_after_is)
                .and_then(|d| if d.0.is_empty() { None } else { Some(d) });
            Declare::new(name, name_span, value)
                .with_params(params)
                .with_doc(doc)
        },
    );

    let decl = choice((decl_has, decl_is));

    doc_comment()
        .or_not()
        .then_ignore(just(Token::Newline).repeated())
        .then(decl)
        .map(|(doc_before, mut decl)| {
            // Only set doc from before if decl doesn't already have one
            if decl.doc_comment().is_none() {
                let doc = doc_before.and_then(|d| if d.0.is_empty() { None } else { Some(d) });
                decl = decl.with_doc(doc);
            }
            decl
        })
}
