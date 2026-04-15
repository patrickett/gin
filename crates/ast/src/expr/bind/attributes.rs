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
    fn is_current_host(&self) -> bool {
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
}

impl ArchTarget {
    fn is_current_host(&self) -> bool {
        match self {
            ArchTarget::X86_64 => cfg!(target_arch = "x86_64"),
            ArchTarget::Arm64 => cfg!(target_arch = "aarch64"),
            ArchTarget::Wasm32 => cfg!(target_arch = "wasm32"),
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
}
