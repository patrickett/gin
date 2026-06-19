use crate::expr::{Expr, Typed};
use internment::Intern;

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

/// A single item inside `#[...]` — either a function call or a bare identifier flag.
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
    /// Time complexity annotation (`#[complexity(...)]`). `None` means unannotated.
    pub complexity: Option<Complexity>,
    /// Raw parsed attributes before semantic extraction.
    /// `None` means no `#[...]` block was present at all.
    /// `Some(vec![])` means an empty `#[]` was present.
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
