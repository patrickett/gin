use crate::expr::{Expr, Literal, Typed};
use internment::Intern;

/// Target operating systems for `#[os({ ... })]` cfg filters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OsTarget {
    Linux,
    MacOS,
    Windows,
    Unknown,
}

impl OsTarget {
    pub(crate) fn is_current_host(&self) -> bool {
        match self {
            OsTarget::Linux => cfg!(target_os = "linux"),
            OsTarget::MacOS => cfg!(target_os = "macos"),
            OsTarget::Windows => cfg!(target_os = "windows"),
            OsTarget::Unknown => false,
        }
    }
}

/// Target CPU architectures for `#[arch({ ... })]` cfg filters.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ArchTarget {
    X86_64,
    Arm64,
    Wasm32,
    Unknown,
}

impl ArchTarget {
    pub(crate) fn is_current_host(&self) -> bool {
        match self {
            ArchTarget::X86_64 => cfg!(target_arch = "x86_64"),
            ArchTarget::Arm64 => cfg!(target_arch = "aarch64"),
            ArchTarget::Wasm32 => cfg!(target_arch = "wasm32"),
            ArchTarget::Unknown => false,
        }
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

/// A single item inside `#[...]` — either a function call or a bare identifier flag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AttributeItem {
    /// A call like `os(['linux'])`
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
    /// OS filter: `#[os({ linux, macos })]`. `None` means no filter (included on all platforms).
    pub os: Option<Vec<OsTarget>>,
    /// Arch filter: `#[arch({ x86_64, arm64 })]`. `None` means no filter.
    pub arch: Option<Vec<ArchTarget>>,
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
    /// Returns `true` if this bind should be compiled for the current build host.
    pub fn matches_current_platform(&self) -> bool {
        if let Some(targets) = &self.os
            && !targets.iter().any(|t| t.is_current_host())
        {
            return false;
        }
        if let Some(arches) = &self.arch
            && !arches.iter().any(|a| a.is_current_host())
        {
            return false;
        }
        true
    }

    /// Extract compiler-known intrinsic attributes from `raw_attributes` into typed fields.
    /// Leaves unknown attributes in `raw_attributes` for tooling to consume.
    /// Should be called after parsing, before platform filtering.
    pub fn extract_intrinsic_attributes(&mut self) {
        let Some(items) = &self.raw_attributes else {
            return;
        };
        if items.is_empty() {
            return;
        }

        for item in items {
            match item {
                AttributeItem::Call { name, args, .. } => match name.as_str() {
                    "os" => {
                        self.os = extract_os_targets(args);
                    }
                    "arch" => {
                        self.arch = extract_arch_targets(args);
                    }
                    "complexity" => {
                        self.complexity = extract_complexity(args);
                    }
                    _ => {}
                },
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

pub(crate) fn extract_os_targets(args: &[Typed<Expr>]) -> Option<Vec<OsTarget>> {
    let list = args.first()?;
    let exprs = match &list.value {
        Expr::List(elems) => elems,
        _ => return None,
    };
    let mut targets = Vec::with_capacity(exprs.len());
    for elem in exprs {
        match &elem.value {
            Expr::Lit(Literal::String(s)) if *s == "linux" => targets.push(OsTarget::Linux),
            Expr::Lit(Literal::String(s)) if *s == "macOS" => targets.push(OsTarget::MacOS),
            Expr::Lit(Literal::String(s)) if *s == "windows" => targets.push(OsTarget::Windows),
            _ => targets.push(OsTarget::Unknown),
        }
    }
    Some(targets)
}

pub(crate) fn extract_arch_targets(args: &[Typed<Expr>]) -> Option<Vec<ArchTarget>> {
    let list = args.first()?;
    let exprs = match &list.value {
        Expr::List(elems) => elems,
        _ => return None,
    };
    let mut targets = Vec::with_capacity(exprs.len());
    for elem in exprs {
        match &elem.value {
            Expr::Lit(Literal::String(s)) if *s == "x86_64" => targets.push(ArchTarget::X86_64),
            Expr::Lit(Literal::String(s)) if *s == "arm64" => targets.push(ArchTarget::Arm64),
            Expr::Lit(Literal::String(s)) if *s == "wasm32" => targets.push(ArchTarget::Wasm32),
            _ => targets.push(ArchTarget::Unknown),
        }
    }
    Some(targets)
}

pub(crate) fn extract_complexity(args: &[Typed<Expr>]) -> Option<Complexity> {
    let variant = args.first()?;
    match &variant.value {
        // Bare tag (no parens) — e.g. `Constant`
        Expr::AnonymousTag(n, _) => match n.as_str() {
            "Constant" => Some(Complexity::Constant),
            _ => None,
        },
        Expr::TagCall(tc) => {
            let variant_name = tc.name.as_str();
            let expr = if tc.args.is_empty() {
                None
            } else if tc.args.len() == 1 {
                extract_complexity_expr_from_expr(&tc.args[0].value)
            } else {
                // Multiple positional params — treat as product
                let vars: Vec<Intern<String>> = tc
                    .args
                    .iter()
                    .filter_map(|a| complexity_var_from_expr(&a.value))
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

/// Extract a `ComplexityExpr` from a single expression (e.g. `n` or `rows * cols`).
fn extract_complexity_expr_from_expr(expr: &Expr) -> Option<ComplexityExpr> {
    match expr {
        Expr::FnCall(call) if call.args.is_none() => Some(ComplexityExpr::Var(call.path.root)),
        Expr::AnonymousTag(n, _) => Some(ComplexityExpr::Var(*n)),
        Expr::Binary(bin) => {
            let left = complexity_var_from_expr(&bin.lhs.value)?;
            let right = complexity_var_from_expr(&bin.rhs.value)?;
            match bin.op {
                crate::BinOp::Multiply => Some(ComplexityExpr::Product(vec![left, right])),
                crate::BinOp::Add => Some(ComplexityExpr::Sum(vec![left, right])),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Extract a single variable name from an expression node.
fn complexity_var_from_expr(expr: &Expr) -> Option<Intern<String>> {
    match expr {
        Expr::FnCall(call) if call.args.is_none() => Some(call.path.root),
        Expr::AnonymousTag(n, _) => Some(*n),
        _ => None,
    }
}
