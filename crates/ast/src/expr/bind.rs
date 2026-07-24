use std::collections::HashMap;

use indexmap::IndexMap;
use internment::Intern;

use crate::TypeExpr;
use crate::doc_comment::DocComment;
use crate::expr::{Expr, Typed};
use crate::parameter::{GroupParam, ParamConvention, ParamSlot, Parameters};
use crate::path::ModPath;
use crate::prelude::*;
use crate::span::{SpanId, Spanned};
use crate::ty::Ty;
use crate::ty_state::TyState;

/// Lazily-formatted method name (e.g., "Single(a).method")
pub struct MethodName<'a> {
    receiver: &'a TypeExpr,
    name: Intern<String>,
}

impl std::fmt::Display for MethodName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}.{}", self.receiver, self.name.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    pub doc_comment: Option<DocComment>,
    pub name: Intern<String>,
    pub name_span: SpanId,
    pub params: Option<Parameters>,
    pub param_slots: IndexMap<Intern<String>, ParamSlot>,
    pub param_conventions: IndexMap<Intern<String>, ParamConvention>,
    /// Group annotations: `[mut r T]` or `[r T]`.
    pub group_params: Vec<GroupParam>,
    /// Maps param names to group names for params with `ref[r]` / `mut[r]` syntax.
    pub param_groups: IndexMap<Intern<String>, Intern<String>>,
    pub attributes: BindAttributes,
    pub value: BindValue,
    /// Method receiver — structural [`TypeExpr`].
    pub receiver_type: Option<Box<Spanned<TypeExpr>>>,
    /// Resolved type variables from the receiver, e.g. `Range[x]` → `{x: Int{...}}`.
    /// Populated during type resolution. Empty for non-method binds.
    pub receiver_typevars: HashMap<Intern<String>, TyState>,
    pub return_type_name: Option<Intern<String>>,
    /// Explicit capitalized return type annotation, e.g. `Str` in `foo() Str: expr`.
    /// Structural [`TypeExpr`].
    pub return_tag: Option<Box<Spanned<TypeExpr>>>,
    /// Bound with `:=` instead of `:`. Immutable after evaluation in this scope.
    pub is_constant: bool,
    /// Participates in prepare-time comptime fold/validate (comptime fn or foldable value).
    /// Set by [`comptime_classify::apply_comptime_classification`], not by the parser.
    pub is_compile_time: bool,

    /// Resolved/progressive return type. Populated during analysis.
    /// Replaces `return_type_name` + `return_tag` + the `fn_return_types` side-table.
    pub return_type: TyState,

    /// Explicit type annotation with value args, e.g. `Maybe(3)` in `val Maybe(3): Some(3)`.
    pub type_annotation: Option<(Intern<String>, Vec<Typed<Expr>>)>,
    /// Qualified path for type annotation, e.g. `Maybe.Some` in `val Maybe.Some(3): ...`
    pub type_annotation_qual: Option<Spanned<ModPath>>,
}

impl Bind {
    pub fn new(name: Intern<String>, name_span: SpanId, value: BindValue) -> Self {
        Bind {
            doc_comment: None,
            name,
            name_span,
            params: None,
            param_slots: IndexMap::new(),
            param_conventions: IndexMap::new(),
            group_params: Vec::new(),
            param_groups: IndexMap::new(),
            attributes: BindAttributes::default(),
            value,
            receiver_type: None,
            receiver_typevars: HashMap::new(),
            return_type_name: None,
            return_tag: None,
            is_constant: false,
            is_compile_time: false,
            return_type: TyState::Infer,
            type_annotation: None,
            type_annotation_qual: None,
        }
    }

    pub fn with_return_type_name(mut self, name: Option<Intern<String>>) -> Self {
        self.return_type_name = name;
        self
    }

    pub fn return_type_name(&self) -> Option<&Intern<String>> {
        self.return_type_name.as_ref()
    }

    pub fn with_receiver_type(mut self, receiver_type: Option<Box<Spanned<TypeExpr>>>) -> Self {
        self.receiver_type = receiver_type;
        self
    }

    pub fn with_params(mut self, params: Option<Parameters>) -> Self {
        self.params = params;
        self
    }

    pub fn with_doc(mut self, doc: Option<DocComment>) -> Self {
        self.doc_comment = doc;
        self
    }

    pub fn with_attributes(mut self, attrs: BindAttributes) -> Self {
        self.attributes = attrs;
        self
    }

    pub fn value_mut(&mut self) -> &mut BindValue {
        &mut self.value
    }

    /// Rename the top-level symbol (used when qualifying module definitions).
    pub fn remap_module_symbol(mut self, symbol: Intern<String>) -> Self {
        self.name = symbol;
        self
    }

    pub fn is_method(&self) -> bool {
        self.receiver_type.is_some()
    }

    /// Return the resolved parameter types in declaration order, with their names.
    /// Only populated after [`populate_ast_types`] or analysis has run.
    /// Returns an empty vec if params haven't been resolved yet.
    pub fn resolved_params(&self) -> Vec<(Intern<String>, Ty)> {
        self.param_slots
            .iter()
            .filter_map(|(name, slot)| match &slot.ty {
                crate::TyState::Resolved(ty) => Some((*name, ty.clone())),
                _ => None,
            })
            .collect()
    }

    /// Return the resolved receiver type variables.
    /// Empty if not a method or not yet resolved.
    pub fn resolved_typevars(&self) -> HashMap<Intern<String>, Ty> {
        self.receiver_typevars
            .iter()
            .filter_map(|(k, tv)| match tv {
                crate::TyState::Resolved(ty) => Some((*k, ty.clone())),
                _ => None,
            })
            .collect()
    }

    pub fn receiver_type_surface(&self) -> Option<&Spanned<TypeExpr>> {
        self.receiver_type.as_deref()
    }

    pub fn method_name(&self) -> Option<MethodName<'_>> {
        let sp = self.receiver_type.as_deref()?;
        Some(MethodName {
            receiver: &sp.value,
            name: self.name,
        })
    }
}

impl std::hash::Hash for Bind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.doc_comment.hash(state);
        self.name.hash(state);
        self.name_span.hash(state);
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
        self.receiver_type.hash(state);
        self.return_tag.hash(state);
        self.return_type_name.hash(state);
        self.type_annotation.hash(state);
        self.type_annotation_qual.hash(state);
        self.value.hash(state);
        for (k, v) in &self.param_conventions {
            k.hash(state);
            v.hash(state);
        }
        self.group_params.hash(state);
        for (k, v) in &self.param_groups {
            k.hash(state);
            v.hash(state);
        }
        // Exclude: param_slots, receiver_typevars, return_type (resolved metadata)
    }
}

/// A simple expression over parameter names for complexity annotations.
///
/// Supports single variables (`n`), products (`rows * cols`), and sums (`V + E`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ComplexityExpr {
    /// A single parameter name
    Var(Intern<String>),
    /// Product of parameter names (e.g. `rows * cols`)
    Product(Vec<Intern<String>>),
    /// Sum of parameter names (e.g. `V + E`)
    Sum(Vec<Intern<String>>),
}

impl ComplexityExpr {
    /// Render as a plain string: "n", "rows * cols", "V + E"
    pub fn render(&self) -> String {
        match self {
            ComplexityExpr::Var(v) => v.as_str().to_string(),
            ComplexityExpr::Product(vars) => vars
                .iter()
                .map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" * "),
            ComplexityExpr::Sum(vars) => vars
                .iter()
                .map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(" + "),
        }
    }

    /// Render wrapped in parens if compound: "n", "(rows * cols)"
    pub fn render_grouped(&self) -> String {
        match self {
            ComplexityExpr::Var(v) => v.as_str().to_string(),
            _ => format!("({})", self.render()),
        }
    }
}

/// Time complexity annotation for `#[complexity(...)]` attributes.
///
/// Used to document the algorithmic cost of a function in big-O notation.
/// The expression parameter (e.g. the `n` in `Linear(n)`) references the
/// author's chosen parameter name(s). Supports compound expressions like
/// `Linear(rows * cols)` and `Linear(V + E)`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Complexity {
    /// O(1) — constant time
    Constant,
    /// O(log expr) — logarithmic
    Logarithmic(ComplexityExpr),
    /// O(expr) — linear
    Linear(ComplexityExpr),
    /// O(expr log expr) — linearithmic
    LogLinear(ComplexityExpr),
    /// O(expr²) — quadratic
    Quadratic(ComplexityExpr),
    /// O(expr³) — cubic
    Cubic(ComplexityExpr),
    /// O(2^expr) — exponential
    Exponential(ComplexityExpr),
    /// O(expr!) — factorial
    Factorial(ComplexityExpr),
}

impl Complexity {
    /// Render the complexity as `Variant(expr)` format (e.g. `Linear(len)`,
    /// `Quadratic(rows * cols)`).
    pub fn display_label(&self) -> String {
        match self {
            Complexity::Constant => "Constant".to_string(),
            Complexity::Logarithmic(expr) => format!("Logarithmic({})", expr.render()),
            Complexity::Linear(expr) => format!("Linear({})", expr.render()),
            Complexity::LogLinear(expr) => format!("LogLinear({})", expr.render()),
            Complexity::Quadratic(expr) => format!("Quadratic({})", expr.render()),
            Complexity::Cubic(expr) => format!("Cubic({})", expr.render()),
            Complexity::Exponential(expr) => format!("Exponential({})", expr.render()),
            Complexity::Factorial(expr) => format!("Factorial({})", expr.render()),
        }
    }

    /// Render the complexity as standard big-O notation (e.g. `O(len)`,
    /// `O(rows * cols)`, `O((rows * cols)²)`). Compound expressions are
    /// wrapped in parens where needed by the variant's notation.
    pub fn display_big_o(&self) -> String {
        match self {
            Complexity::Constant => "O(1)".to_string(),
            Complexity::Logarithmic(expr) => format!("O(log {})", expr.render_grouped()),
            Complexity::Linear(expr) => format!("O({})", expr.render()),
            Complexity::LogLinear(expr) => {
                format!("O({} log {})", expr.render(), expr.render_grouped())
            }
            Complexity::Quadratic(expr) => format!("O({}²)", expr.render_grouped()),
            Complexity::Cubic(expr) => format!("O({}³)", expr.render_grouped()),
            Complexity::Exponential(expr) => format!("O(2^{})", expr.render_grouped()),
            Complexity::Factorial(expr) => format!("O({}!)", expr.render_grouped()),
        }
    }
}

/// A parsed `#attr` item.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AttributeItem {
    /// A call like `complexity(Linear(n))`
    Call {
        name: Intern<String>,
        name_span: crate::span::SpanId,
        args: Vec<Typed<Expr>>,
    },
    /// A bare identifier like `debug`, `test`, `inline`
    Flag {
        name: Intern<String>,
        span: crate::span::SpanId,
    },
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct BindAttributes {
    /// Always run in tests (`#[test]`).
    pub test: bool,
    /// Always inline (`#[inline]`).
    pub inline_always: bool,
    /// Strip in release builds (`#[debug]`).
    pub debug_only: bool,
    /// Time complexity annotation (`#complexity(...)`). `None` means unannotated.
    pub complexity: Option<Complexity>,
    /// Raw parsed attributes before semantic extraction.
    /// `None` means no attributes were present.
    pub raw_attributes: Option<Vec<AttributeItem>>,
}

impl BindAttributes {
    /// Extract compiler-known intrinsic attributes from `raw_attributes` into typed fields.
    /// Leaves unknown attributes in `raw_attributes` for tooling to consume.
    /// Should be called after parsing.
    pub fn extract_intrinsic_attributes(&mut self) {
        let Some(items) = &self.raw_attributes else {
            return;
        };
        if items.is_empty() {
            return;
        }

        for item in items {
            match item {
                AttributeItem::Call { name, args, .. } => {
                    if name.as_str() == "complexity" {
                        self.complexity = Complexity::extract(args);
                    }
                }
                AttributeItem::Flag { name, .. } => match name.as_str() {
                    "debug" => self.debug_only = true,
                    "test" => self.test = true,
                    "inline" => self.inline_always = true,
                    _ => {}
                },
            }
        }
    }
}

impl Complexity {
    /// Extract a complexity annotation from parsed attribute arguments.
    pub(crate) fn extract(args: &[Typed<Expr>]) -> Option<Complexity> {
        let variant = args.first()?;
        match &variant.value {
            // Bare tag (no parens) — e.g. `Constant`
            Expr::AnonymousTag(n) => match n.as_str() {
                "Constant" => Some(Complexity::Constant),
                _ => None,
            },
            Expr::TagCall(tc) => {
                let variant_name = tc.name.as_str();
                let expr = if tc.args.is_empty() {
                    None
                } else if tc.args.len() == 1 {
                    tc.args[0].value.extract_complexity_expr()
                } else {
                    // Multiple positional params — treat as product
                    let vars: Vec<Intern<String>> = tc
                        .args
                        .iter()
                        .filter_map(|a| a.value.complexity_var())
                        .collect();
                    if vars.is_empty() {
                        None
                    } else {
                        Some(ComplexityExpr::Product(vars))
                    }
                };
                match (variant_name, expr) {
                    ("Constant", _) => Some(Complexity::Constant),
                    ("Logarithmic", Some(e)) => Some(Complexity::Logarithmic(e)),
                    ("Linear", Some(e)) => Some(Complexity::Linear(e)),
                    ("LogLinear", Some(e)) => Some(Complexity::LogLinear(e)),
                    ("Quadratic", Some(e)) => Some(Complexity::Quadratic(e)),
                    ("Cubic", Some(e)) => Some(Complexity::Cubic(e)),
                    ("Exponential", Some(e)) => Some(Complexity::Exponential(e)),
                    ("Factorial", Some(e)) => Some(Complexity::Factorial(e)),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

impl Expr {
    /// Extract a `ComplexityExpr` from this expression (e.g. `n` or `rows * cols`).
    fn extract_complexity_expr(&self) -> Option<ComplexityExpr> {
        match self {
            Expr::FnCall(call) if call.args.is_none() => Some(ComplexityExpr::Var(call.path.root)),
            Expr::AnonymousTag(n) => Some(ComplexityExpr::Var(*n)),
            Expr::Binary(bin) => {
                let left = bin.lhs.value.complexity_var()?;
                let right = bin.rhs.value.complexity_var()?;
                match bin.op {
                    crate::BinOp::Multiply => Some(ComplexityExpr::Product(vec![left, right])),
                    crate::BinOp::Add => Some(ComplexityExpr::Sum(vec![left, right])),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Extract a single variable name from this expression node.
    fn complexity_var(&self) -> Option<Intern<String>> {
        match self {
            Expr::FnCall(call) if call.args.is_none() => Some(call.path.root),
            Expr::AnonymousTag(n) => Some(*n),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BindValue {
    Expr(Box<Typed<Expr>>),
    Body {
        exprs: Vec<Typed<Expr>>,
        ret: Return,
    },
    /// External function declaration — no body, provided by the C runtime or linker.
    Extern,
    /// A variable declared with a type but no assigned value yet.
    /// e.g. `value Int` (on its own line) declares `value` of type `Int` without assignment.
    /// Assignment happens later via `name: value` which becomes `TypedExprKind::Reassign`.
    Unassigned,
}
