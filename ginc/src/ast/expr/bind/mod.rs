use crate::codegen::prelude::*;
use crate::diagnostic::codegen::CodegenSymptom;
use crate::{ast::tag::Tag, codegen::lower_function, parse::block, prelude::*};

mod attributes;
mod value;
pub use attributes::*;
pub use value::*;

/// Lazily-formatted method name (e.g., "Single(a).method")
pub struct MethodName<'a> {
    // TODO: come up with a better name than receiver
    receiver: &'a Tag,
    name: IStr,
}

impl std::fmt::Display for MethodName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.receiver {
            Tag::Nominal(type_name) => {
                write!(f, "{}.{}", type_name.as_str(), self.name.as_str())
            }
            Tag::Generic(type_name, params) => {
                write!(f, "{}(", type_name.as_str())?;
                for (i, (k, v)) in params.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}: {}", k.as_str(), v)?;
                }
                write!(f, ").{}", self.name.as_str())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    doc_comment: Option<DocComment>,
    attributes: BindAttributes,
    name: IStr,
    params: Option<Parameters>,
    value: BindValue,
    receiver_type: Option<Tag>,
}

impl Bind {
    pub fn new(name: IStr, value: BindValue) -> Self {
        Bind {
            doc_comment: None,
            attributes: BindAttributes::default(),
            name,
            params: None,
            value,
            receiver_type: None,
        }
    }

    pub fn with_params(mut self, params: Option<Parameters>) -> Self {
        self.params = params;
        self
    }

    pub fn with_receiver_type(mut self, receiver_type: Option<Tag>) -> Self {
        self.receiver_type = receiver_type;
        self
    }

    pub fn name(&self) -> IStr {
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

    pub fn infer_return_type(&self) -> Option<String> {
        match &self.value {
            BindValue::Expr(expr) => infer_expr_type(expr, &[]),
            BindValue::Body { exprs, ret } => match &ret.0 {
                None => Some("Nothing".to_string()),
                Some(expr) => infer_expr_type(expr, exprs),
            },
        }
    }
}

fn infer_expr_type(expr: &Expr, locals: &[Expr]) -> Option<String> {
    match expr {
        Expr::Lit(lit) => Some(
            match lit {
                Literal::Int(_) | Literal::Number(_) => "Int",
                Literal::Float(_) => "Float",
                Literal::String(_) => "String",
            }
            .to_string(),
        ),
        Expr::FormatString(_) => Some("String".to_string()),
        Expr::Binary(b) if b.op.is_comparison() => Some("Bool".to_string()),
        Expr::Binary(b) => infer_expr_type(&b.lhs, locals).or_else(|| infer_expr_type(&b.rhs, locals)),
        Expr::FnCall(call) if call.args.is_none() && call.path.segments.is_empty() => {
            let name = call.path.root.as_str();
            locals.iter().find_map(|e| match e {
                Expr::Bind(b) if b.name().as_str() == name => b.infer_return_type(),
                _ => None,
            })
        }
        _ => None,
    }
}

impl std::hash::Hash for Bind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
        self.receiver_type.hash(state);
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
pub fn bind<'t, I>(
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let params = params(expr.clone(), tag(expr.clone()));

    use Token::*;

    let lhs = id_token()
        .then(params.or_not())
        .then(tag(expr.clone()).or_not())
        .then_ignore(just(Token::Colon));

    let single = lhs
        .clone()
        .then(expr.clone())
        .map(|(((name, params), _opt_tag), rhs)| {
            // TODO: do something with optional return type
            Bind::new(name, BindValue::Expr(Box::new(rhs))).with_params(params)
        });

    let open = just(Newline);
    let body = expr.clone();
    let close = r#return(expr.clone());

    let multiple = lhs.then(block(open, body, close)).map(
        |(((name, params), _opt_tag), (_nl, exprs, ret))| {
            Bind::new(name, BindValue::Body { exprs, ret }).with_params(params)
        },
    );

    let bind = choice((multiple, single));

    doc_comment().or_not().then(bind).map(|(doc, bind)| {
        let doc = doc.and_then(|d| if d.0.is_empty() { None } else { Some(d) });
        bind.with_doc(doc)
    })
}

impl<'c> Lower<'c> for Bind {
    fn lower(
        &self,
        ctx: &CodegenContext<'_, 'c>,
        block: &BlockRef<'c, 'c>,
        symtab: &mut RuntimeSymbolTable<'c>,
    ) -> Result<Value<'c, 'c>, CodegenSymptom> {
        match &self.value() {
            BindValue::Body { exprs: _, ret: _ } => {
                let func_op = lower_function(ctx, &self.name(), self)?;
                block.append_operation(func_op);

                // Return a placeholder value (TODO: consider returning function reference)
                Ok(block.const_i64(ctx.mlir, 0))
            }
            BindValue::Expr(expr) => {
                let value = expr.lower(ctx, block, symtab)?;
                symtab.insert(self.name().as_str().to_string(), value);
                Ok(value)
            }
        }
    }
}
