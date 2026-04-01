use crate::{block, prelude::*, tag::Tag};
use chumsky::span::SimpleSpan;
use internment::Intern;
use lexer::Token;

mod attributes;
mod value;
pub use attributes::*;
pub use value::*;

/// Lazily-formatted method name (e.g., "Single(a).method")
pub struct MethodName<'a> {
    // TODO: come up with a better name than receiver
    receiver: &'a Tag,
    name: Intern<String>,
}

impl std::fmt::Display for MethodName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.receiver {
            Tag::Nominal(type_name, _) => {
                write!(f, "{}.{}", type_name.as_str(), self.name.as_str())
            }
            Tag::Generic(type_name, params, _) => {
                write!(f, "{}(", type_name.as_str())?;
                for (i, (k, v)) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k.as_str(), v)?;
                }
                write!(f, ").{}", self.name.as_str())
            }
            Tag::Qualified(path) => {
                write!(f, "{}", path.root.as_str())?;
                for seg in &path.segments {
                    write!(f, ".{}", seg.as_str())?;
                }
                write!(f, ".{}", self.name.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    doc_comment: Option<DocComment>,
    attributes: BindAttributes,
    name: Intern<String>,
    pub name_span: SimpleSpan,
    params: Option<Parameters>,
    value: BindValue,
    receiver_type: Option<Tag>,
    return_type_name: Option<Intern<String>>,
    /// Explicit capitalized return type annotation, e.g. `Str` in `foo() Str: expr`.
    pub return_tag: Option<Tag>,
    /// Explicit type annotation with value args, e.g. `Maybe(3)` in `val Maybe(3): Some(3)`.
    pub type_annotation: Option<(Intern<String>, Vec<Spanned<Expr>>)>,
    /// Qualified path for type annotation, e.g. `Maybe.Some` in `val Maybe.Some(3): ...`
    pub type_annotation_qual: Option<ModPath>,
    /// `true` for `:=` (immutable/const) binds; `false` for `:` (mutable, alloca-backed) binds.
    pub is_const: bool,
}

impl Bind {
    pub fn new(
        name: Intern<String>,
        name_span: SimpleSpan,
        value: BindValue,
        is_const: bool,
    ) -> Self {
        Bind {
            doc_comment: None,
            attributes: BindAttributes::default(),
            name,
            name_span,
            params: None,
            value,
            receiver_type: None,
            return_type_name: None,
            return_tag: None,
            type_annotation: None,
            type_annotation_qual: None,
            is_const,
        }
    }

    pub fn with_return_type_name(mut self, name: Option<Intern<String>>) -> Self {
        self.return_type_name = name;
        self
    }

    pub fn return_type_name(&self) -> Option<&Intern<String>> {
        self.return_type_name.as_ref()
    }

    pub fn with_params(mut self, params: Option<Parameters>) -> Self {
        self.params = params;
        self
    }

    pub fn with_receiver_type(mut self, receiver_type: Option<Tag>) -> Self {
        self.receiver_type = receiver_type;
        self
    }

    pub fn name(&self) -> Intern<String> {
        self.name
    }

    pub fn params(&self) -> &Option<Parameters> {
        &self.params
    }

    pub fn with_doc(mut self, doc: Option<DocComment>) -> Self {
        self.doc_comment = doc;
        self
    }

    pub fn doc_comment(&self) -> Option<&DocComment> {
        self.doc_comment.as_ref()
    }

    pub fn with_attributes(mut self, attrs: BindAttributes) -> Self {
        self.attributes = attrs;
        self
    }

    pub fn attributes(&self) -> &BindAttributes {
        &self.attributes
    }

    pub fn value(&self) -> &BindValue {
        &self.value
    }

    pub fn is_method(&self) -> bool {
        self.receiver_type.is_some()
    }

    pub fn receiver_type(&self) -> Option<&Tag> {
        self.receiver_type.as_ref()
    }

    pub fn method_name(&self) -> Option<MethodName<'_>> {
        self.receiver_type.as_ref().map(|t| MethodName {
            receiver: t,
            name: self.name,
        })
    }
}

impl std::hash::Hash for Bind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
        self.is_const.hash(state);
        self.receiver_type.hash(state);
        self.return_type_name.hash(state);
        // Hash params manually since HashMap doesn't impl Hash
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

// TODO: it would be cool if we could just impl Parse on T
// impl Parse for Bind { ... }
/// Parse a `#[attr, attr, ...]` attribute block.
///
/// Each attribute item is one of:
/// - `os({ linux, macos, windows, unknown })` — OS filter
/// - `arch({ x86_64, arm64, wasm32 })` — arch filter
/// - `debug` — strip in release builds
/// - `test` — only compiled / run in test mode
/// - `inline` — hint to always inline
fn bind_attributes<'t, I>() -> impl Parser<'t, I, BindAttributes, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let os_name = select! {
        Token::Id("linux")   => OsTarget::Linux,
        Token::Id("macos")   => OsTarget::MacOS,
        Token::Id("windows") => OsTarget::Windows,
        Token::Id("unknown") => OsTarget::Unknown,
    };

    let arch_name = select! {
        Token::Id("x86_64") => ArchTarget::X86_64,
        Token::Id("arm64")  => ArchTarget::Arm64,
        Token::Id("wasm32") => ArchTarget::Wasm32,
    };

    let os_set = just(Token::CurlyOpen)
        .ignore_then(
            os_name
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::CurlyClose));

    let arch_set = just(Token::CurlyOpen)
        .ignore_then(
            arch_name
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::CurlyClose));

    let os_attr = just(Token::Id("os"))
        .ignore_then(just(Token::ParenOpen))
        .ignore_then(os_set)
        .then_ignore(just(Token::ParenClose))
        .map(|targets| BindAttributes {
            os: Some(targets),
            ..Default::default()
        });

    let arch_attr = just(Token::Id("arch"))
        .ignore_then(just(Token::ParenOpen))
        .ignore_then(arch_set)
        .then_ignore(just(Token::ParenClose))
        .map(|targets| BindAttributes {
            arch: Some(targets),
            ..Default::default()
        });

    let debug_attr = just(Token::Id("debug")).map(|_| BindAttributes {
        debug_only: true,
        ..Default::default()
    });

    let test_attr = just(Token::Id("test")).map(|_| BindAttributes {
        test: true,
        ..Default::default()
    });

    let inline_attr = just(Token::Id("inline")).map(|_| BindAttributes {
        inline_always: true,
        ..Default::default()
    });

    let attr_item = choice((os_attr, arch_attr, debug_attr, test_attr, inline_attr));

    just(Token::Pound)
        .ignore_then(just(Token::BracketOpen))
        .ignore_then(
            attr_item
                .separated_by(just(Token::Comma))
                .allow_trailing()
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(Token::BracketClose))
        .map(|items| {
            items
                .into_iter()
                .fold(BindAttributes::default(), |mut acc, item| {
                    if item.os.is_some() {
                        acc.os = item.os;
                    }
                    if item.arch.is_some() {
                        acc.arch = item.arch;
                    }
                    if item.debug_only {
                        acc.debug_only = true;
                    }
                    if item.test {
                        acc.test = true;
                    }
                    if item.inline_always {
                        acc.inline_always = true;
                    }
                    acc
                })
        })
}

pub fn bind<'t, I>(
    expr: impl Parser<'t, I, Spanned<Expr>, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let params = params(expr.clone(), tag(expr.clone()));

    use Token::*;

    type ReturnTypePart = (
        Option<Intern<std::string::String>>,
        Option<crate::Tag>,
        Option<(Intern<std::string::String>, Vec<Spanned<Expr>>)>,
        Option<ModPath>, // Qualified path for type annotation
    );

    // Parses optional return-type hint before the colon.
    // lowercase id        → named union return type  (e.g., `print(a) result:`)
    // Capitalized(expr..) → type annotation with value args  (e.g., `val Maybe(3):`)
    // Capitalized         → explicit type annotation  (e.g., `foo(n Int) Str:`)
    // Qualified.Capitalized → qualified type annotation (e.g., `foo() Bool.True:`)
    // Qualified.Capitalized(expr..) → qualified type with args (e.g., `val Maybe.Some(3):`)
    let return_type_part = choice((
        select! { Token::Id(name) => Intern::<std::string::String>::new(name.to_string()) }
            .map(|n| -> ReturnTypePart { (Some(n), None, None, None) }),
        // Qualified type path with args: Maybe.Some(3), Bool.True
        crate::tag_variant_path()
            .then(
                crate::delimited_list(
                    Token::ParenOpen,
                    expr.clone(),
                    Token::Comma,
                    Token::ParenClose,
                )
                .or_not(),
            )
            .map(|(path, args)| -> ReturnTypePart {
                match args {
                    Some(args) if !args.is_empty() => {
                        // For Maybe.Some(3), use the last segment as the name
                        let variant_name = *path.segments.last().unwrap_or(&path.root);
                        (None, None, Some((variant_name, args)), Some(path))
                    }
                    _ => (None, Some(crate::Tag::Qualified(path)), None, None),
                }
            })
            .boxed(),
        select! { Token::Tag(name) => Intern::<std::string::String>::new(name.to_string()) }
            .then(
                crate::delimited_list(
                    Token::ParenOpen,
                    expr.clone(),
                    Token::Comma,
                    Token::ParenClose,
                )
                .or_not(),
            )
            .map_with(|(name, args), e| -> ReturnTypePart {
                match args {
                    Some(args) if !args.is_empty() => (None, None, Some((name, args)), None),
                    _ => (None, Some(crate::Tag::Nominal(name, e.span())), None, None),
                }
            }),
    ))
    .or_not()
    .map(|opt| -> ReturnTypePart { opt.unwrap_or((None, None, None, None)) });

    // `:=` → const bind (immutable SSA value)
    // `:`  → mutable bind (alloca-backed)
    let bind_op = choice((
        just(Token::ColonEq).map(|_| true),
        just(Token::Colon).map(|_| false),
    ));

    let lhs = id_token()
        .map_with(|name, e| (name, e.span()))
        .then(params.or_not())
        .then(return_type_part)
        .then(bind_op);

    let extern_value = just(Token::Extern).map(|_| (BindValue::Extern, None::<crate::DocComment>));

    let single_value = expr
        .clone()
        .then(doc_comment().or_not())
        .map(|(e, doc)| (BindValue::Expr(Box::new(e)), doc));

    let open = just(Newline);
    let body = expr.clone();
    let close = r#return(expr.clone());

    let multi_value =
        block(open, body, close).map(|(_nl, exprs, ret)| (BindValue::Body { exprs, ret }, None));

    let bind = lhs
        .then(choice((extern_value, multi_value, single_value)))
        .map(
            |(
                (
                    (
                        ((name, name_span), params),
                        (return_type_name, return_tag, type_annotation, type_annotation_qual),
                    ),
                    is_const,
                ),
                (value, postfix_doc),
            )| {
                let mut b = Bind::new(name, name_span, value, is_const)
                    .with_params(params)
                    .with_return_type_name(return_type_name);
                b.return_tag = return_tag;
                b.type_annotation = type_annotation;
                b.type_annotation_qual = type_annotation_qual;
                if let Some(doc) =
                    postfix_doc.and_then(|d| if d.0.is_empty() { None } else { Some(d) })
                {
                    b = b.with_doc(Some(doc));
                }
                b
            },
        );

    bind_attributes()
        .then_ignore(just(Token::Newline).repeated())
        .or_not()
        .then(
            doc_comment()
                .or_not()
                .then_ignore(just(Token::Newline).repeated())
                .then_ignore(just(Token::Indent).or_not())
                .then(bind),
        )
        .map(|(attrs, (doc_before, bind))| {
            let bind = if bind.doc_comment().is_none() {
                let doc = doc_before.and_then(|d| if d.0.is_empty() { None } else { Some(d) });
                bind.with_doc(doc)
            } else {
                bind
            };
            if let Some(attrs) = attrs {
                bind.with_attributes(attrs)
            } else {
                bind
            }
        })
}
