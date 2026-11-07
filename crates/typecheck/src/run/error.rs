use i256::I256;

/// Why a run stopped. Every variant but [`Error::DomainViolated`] is about the program asked
/// for or the interpreter's reach; `DomainViolated` is a bug in the checker (or the recorder).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("no function named `{0}`")]
    NoSuchFunction(String),
    #[error("`{function}` takes {expected} arguments, {given} given")]
    Arity {
        function: String,
        expected: usize,
        given: usize,
    },
    #[error("`{function}` takes {expected} type arguments, {given} given")]
    TypeArity {
        function: String,
        expected: usize,
        given: usize,
    },
    /// The checker did not record a body for it (a construct it does not record yet), or it
    /// is a machine instruction, which has none.
    #[error("no executable body was recorded for `{0}`")]
    NoBody(String),
    #[error("{what} is not supported by the interpreter yet (in `{function}`)")]
    Unsupported { what: &'static str, function: String },
    #[error("`{function}` was given {value} for a parameter of type {lower} to {upper}")]
    ArgumentOutOfDomain {
        function: String,
        value: I256,
        lower: I256,
        upper: I256,
    },
    /// A value left the domain the checker proved for its node. The checker or the recorder
    /// is wrong; the program is not at fault.
    #[error("checker bug: `{function}` node {node} computed {value}, outside the proved {lower} to {upper}")]
    DomainViolated {
        function: String,
        node: u32,
        value: I256,
        lower: I256,
        upper: I256,
    },
    /// The recorded body is not shaped as the interpreter expects.
    #[error("recorded body of `{function}` is inconsistent at node {node}: {what}")]
    Inconsistent {
        function: String,
        node: u32,
        what: String,
    },
    /// A `#todo` hole was reached. `stack` is the call chain, outermost first, ending in the
    /// function that holds the hole.
    #[error("reached a `#todo` hole in `{function}` (called from {})", .stack.join(" -> "))]
    HoleReached { function: String, node: u32, stack: Vec<String> },
    #[error("ran out of fuel after {0} steps (an infinite loop?)")]
    OutOfFuel(u64),
    #[error("call depth limit of {0} exceeded")]
    DepthExceeded(usize),
    #[error("a bound call `{method}` on `{ty}` found no such method")]
    BoundNotSatisfied { ty: String, method: String },
}
