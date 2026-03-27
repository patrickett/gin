use crate::codegen::prelude::*;
use crate::codegen::ty_to_mlir;
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
    name: IStr,
    pub name_span: SimpleSpan,
    params: Option<Parameters>,
    value: BindValue,
    receiver_type: Option<Tag>,
    return_type_name: Option<IStr>,
    /// Explicit capitalized return type annotation, e.g. `Str` in `foo() Str: expr`.
    pub return_tag: Option<Tag>,
    /// Explicit type annotation with value args, e.g. `Maybe(3)` in `val Maybe(3): Some(3)`.
    pub type_annotation: Option<(IStr, Vec<Expr>)>,
    /// Qualified path for type annotation, e.g. `Maybe.Some` in `val Maybe.Some(3): ...`
    pub type_annotation_qual: Option<ModPath>,
    /// `true` for `:=` (immutable/const) binds; `false` for `:` (mutable, alloca-backed) binds.
    pub is_const: bool,
}

impl Bind {
    pub fn new(name: IStr, name_span: SimpleSpan, value: BindValue, is_const: bool) -> Self {
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

    pub fn with_return_type_name(mut self, name: Option<IStr>) -> Self {
        self.return_type_name = name;
        self
    }

    pub fn return_type_name(&self) -> Option<&IStr> {
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

    pub fn infer_return_type(&self) -> Option<String> {
        self.infer_return_type_inner(None, &mut std::collections::HashSet::new())
    }

    pub fn infer_return_type_with_defs(&self, defs: &crate::DefMap) -> Option<String> {
        self.infer_return_type_inner(Some(defs), &mut std::collections::HashSet::new())
    }

    fn infer_return_type_inner(
        &self,
        defs: Option<&crate::DefMap>,
        visited: &mut std::collections::HashSet<IStr>,
    ) -> Option<String> {
        match &self.value {
            BindValue::Expr(expr) => infer_expr_type(expr, &[], defs, visited),
            BindValue::Body { exprs, ret } => match &ret.0 {
                None => Some("Nothing".to_string()),
                Some(expr) => infer_expr_type(expr, exprs, defs, visited),
            },
            BindValue::Extern => None,
        }
    }

    /// Infer the return type as a union of anonymous tags.
    ///
    /// Returns the named return type if one exists, otherwise extracts all
    /// anonymous tags from the bind's return value and formats them as a union.
    /// If there are no anonymous tags, falls back to expression-based type inference.
    pub fn infer_return_type_union(&self) -> Option<String> {
        if let Some(name) = self.return_type_name() {
            return Some(name.to_string());
        }

        let tags = self.extract_anonymous_tags();
        if tags.is_empty() {
            self.infer_return_type()
        } else {
            let unique_tags: std::collections::HashSet<_> = tags.into_iter().collect();
            let tag_strings: Vec<String> = unique_tags.into_iter().map(|t| t.to_string()).collect();
            Some(tag_strings.join(" or "))
        }
    }

    /// Like `infer_return_type_union` but with module-level defs for transitive inference.
    pub fn infer_return_type_union_with_defs(&self, defs: &crate::DefMap) -> Option<String> {
        if let Some(name) = self.return_type_name() {
            return Some(name.to_string());
        }

        let tags = self.extract_anonymous_tags();
        if !tags.is_empty() {
            let unique_tags: std::collections::HashSet<_> = tags.into_iter().collect();
            let mut tag_strings: Vec<String> =
                unique_tags.into_iter().map(|t| t.to_string()).collect();
            tag_strings.sort();
            return Some(tag_strings.join(" or "));
        }

        if let BindValue::Body { exprs, ret } = &self.value {
            let types = collect_all_return_types(
                exprs,
                ret,
                Some(defs),
                &mut std::collections::HashSet::new(),
            );
            if types.len() > 1 {
                let unique: std::collections::HashSet<_> = types.into_iter().collect();
                let mut sorted: Vec<String> = unique.into_iter().collect();
                sorted.sort();
                return Some(sorted.join(" or "));
            }
        }

        self.infer_return_type_with_defs(defs)
    }

    /// Extract all anonymous tag names from this bind's return value.
    fn extract_anonymous_tags(&self) -> Vec<IStr> {
        let mut tags = Vec::new();

        match &self.value {
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
}

/// Collect all return type strings from a function body, including early returns
/// inside if/when/loop expressions.
fn collect_all_return_types(
    exprs: &[Expr],
    ret: &crate::ast::expr::r#return::Return,
    defs: Option<&crate::DefMap>,
    visited: &mut std::collections::HashSet<IStr>,
) -> Vec<String> {
    let mut types = Vec::new();

    match &ret.0 {
        None => types.push("Nothing".to_string()),
        Some(expr) => {
            if let Some(ty) = infer_expr_type(expr, exprs, defs, visited) {
                types.push(ty);
            }
        }
    }

    for expr in exprs {
        collect_early_return_types(expr, exprs, defs, visited, &mut types);
    }

    types
}

fn collect_early_return_types(
    expr: &Expr,
    locals: &[Expr],
    defs: Option<&crate::DefMap>,
    visited: &mut std::collections::HashSet<IStr>,
    types: &mut Vec<String>,
) {
    if let Expr::If(if_expr) = expr {
        for body_expr in &if_expr.body {
            collect_early_return_types(body_expr, locals, defs, visited, types);
        }
        match &if_expr.ret.0 {
            None => types.push("Nothing".to_string()),
            Some(ret_expr) => {
                if let Some(ty) = infer_expr_type(ret_expr, locals, defs, visited) {
                    types.push(ty);
                }
            }
        }
    }
}

fn infer_expr_type(
    expr: &Expr,
    locals: &[Expr],
    defs: Option<&crate::DefMap>,
    visited: &mut std::collections::HashSet<IStr>,
) -> Option<String> {
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
        Expr::Binary(b) => infer_expr_type(&b.lhs, locals, defs, visited)
            .or_else(|| infer_expr_type(&b.rhs, locals, defs, visited)),
        Expr::FnCall(call) if call.path.segments.is_empty() => {
            let name = call.path.root.as_str();
            let from_locals = locals.iter().find_map(|e| match e {
                Expr::Bind(b) if b.name().as_str() == name => {
                    b.infer_return_type_inner(defs, visited)
                }
                _ => None,
            });
            from_locals.or_else(|| {
                defs.and_then(|defs| {
                    defs.values().find_map(|b| {
                        if b.name().as_str() == name {
                            if visited.contains(&b.name()) {
                                return None; // cycle guard
                            }
                            visited.insert(b.name());
                            b.infer_return_type_inner(Some(defs), visited)
                        } else {
                            None
                        }
                    })
                })
            })
        }
        Expr::AnonymousTag(name, _) => Some(name.to_string()),
        Expr::TagCall(_) => None,
        _ => None,
    }
}

/// Recursively extract anonymous tag names from an expression.
fn extract_anonymous_tags_from_expr(expr: &Expr, tags: &mut Vec<IStr>) {
    use crate::ast::expr::Loop;

    match expr {
        // TagCall returns a union type, not an anonymous tag - don't extract from it
        Expr::AnonymousTag(name, _) => {
            tags.push(*name);
        }
        Expr::FnCall(call) => {
            if let Some(args) = &call.args {
                for arg in args {
                    extract_anonymous_tags_from_expr(arg, tags);
                }
            }
        }
        Expr::Binary(bin) => {
            extract_anonymous_tags_from_expr(&bin.lhs, tags);
            extract_anonymous_tags_from_expr(&bin.rhs, tags);
        }
        Expr::Loop(loop_expr) => match loop_expr {
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
        },
        Expr::When(when_expr) => {
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
        Expr::If(if_expr) => {
            // Only extract from the return statement inside the if block
            // The condition pattern matching doesn't contribute to return type
            if let Some(expr) = &if_expr.ret.0 {
                extract_anonymous_tags_from_expr(expr, tags);
            }
        }
        Expr::Bind(bind) => match bind.value() {
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
        },
        Expr::TupleAlloc { init, .. } => extract_anonymous_tags_from_expr(init, tags),
        Expr::TupleGet { base, .. } => extract_anonymous_tags_from_expr(base, tags),
        Expr::TupleSet { base, value, .. } => {
            extract_anonymous_tags_from_expr(base, tags);
            extract_anonymous_tags_from_expr(value, tags);
        }
        Expr::Cast { expr, .. } => extract_anonymous_tags_from_expr(expr, tags),
        _ => {}
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
    expr: impl Parser<'t, I, Expr, ParserError<'t>> + Clone + 't,
) -> impl Parser<'t, I, Bind, ParserError<'t>> + Clone
where
    I: ValueInput<'t, Token = Token<'t>, Span = SimpleSpan>,
{
    let params = params(expr.clone(), tag(expr.clone()));

    use Token::*;

    type ReturnTypePart = (
        Option<IStr>,
        Option<crate::ast::Tag>,
        Option<(IStr, Vec<Expr>)>,
        Option<ModPath>,  // Qualified path for type annotation
    );

    // Parses optional return-type hint before the colon.
    // lowercase id        → named union return type  (e.g., `print(a) result:`)
    // Capitalized(expr..) → type annotation with value args  (e.g., `val Maybe(3):`)
    // Capitalized         → explicit type annotation  (e.g., `foo(n Int) Str:`)
    // Qualified.Capitalized → qualified type annotation (e.g., `foo() Bool.True:`)
    // Qualified.Capitalized(expr..) → qualified type with args (e.g., `val Maybe.Some(3):`)
    let return_type_part = choice((
        select! { Token::Id(name) => IStr::new(name.to_string()) }
            .map(|n| -> ReturnTypePart { (Some(n), None, None, None) }),
        // Qualified type path with args: Maybe.Some(3), Bool.True
        crate::ast::tag_variant_path()
            .then(
                crate::parse::delimited_list(
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
                    _ => (None, Some(crate::ast::Tag::Qualified(path)), None, None),
                }
            })
            .boxed(),
        select! { Token::Tag(name) => IStr::new(name.to_string()) }
            .then(
                crate::parse::delimited_list(
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
                    _ => (None, Some(crate::ast::Tag::Nominal(name, e.span())), None, None),
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

    let extern_value =
        just(Token::Extern).map(|_| (BindValue::Extern, None::<crate::ast::DocComment>));

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
                ((((name, name_span), params), (return_type_name, return_tag, type_annotation, type_annotation_qual)), is_const),
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
                let name_str = self.name().as_str().to_string();
                if self.is_const {
                    // Const bind (`:=`): direct SSA value in symtab — no alloca.
                    let value = expr.lower(ctx, block, symtab)?;
                    symtab.insert(name_str.clone(), value);
                    let ty = ctx
                        .ty_env
                        .infer_expr(expr, &std::collections::HashMap::new());
                    ctx.var_types.borrow_mut().insert(name_str, ty);
                    Ok(value)
                } else {
                    let loc = ctx.location();
                    if ctx.mutable_slots.borrow().contains(&name_str) {
                        // Rebind (`:`) of an existing mutable variable — store new value.
                        let ptr = *symtab.get(&name_str).ok_or_else(|| {
                            CodegenSymptom::Internal(format!(
                                "mutable slot '{name_str}' not found in symtab"
                            ))
                        })?;
                        let new_val = expr.lower(ctx, block, symtab)?;
                        block.store_typed(ptr, new_val, loc)?;
                        Ok(block.const_i64(ctx.mlir, 0))
                    } else {
                        // First mutable bind (`:`) — alloca + store.
                        // Build locals from var_types so infer_expr can resolve base types
                        // (e.g., elem type of arrays in TupleGet expressions).
                        let locals: std::collections::HashMap<IStr, crate::typeck::Ty> = ctx
                            .var_types
                            .borrow()
                            .iter()
                            .map(|(k, v)| (IStr::new(k.clone()), v.clone()))
                            .collect();
                        let ty = ctx.ty_env.infer_expr(expr, &locals);
                        let elem_mlir_ty = ty_to_mlir(&ty, ctx.mlir);
                        let slot = block.alloca_typed(ctx.mlir, elem_mlir_ty, loc);
                        let init_val = expr.lower(ctx, block, symtab)?;
                        block.store_typed(slot, init_val, loc)?;
                        symtab.insert(name_str.clone(), slot);
                        ctx.var_types.borrow_mut().insert(name_str.clone(), ty);
                        ctx.mutable_slots.borrow_mut().insert(name_str);
                        Ok(slot)
                    }
                }
            }
            BindValue::Extern => {
                let func_op = lower_function(ctx, &self.name(), self)?;
                block.append_operation(func_op);
                Ok(block.const_i64(ctx.mlir, 0))
            }
        }
    }
}
