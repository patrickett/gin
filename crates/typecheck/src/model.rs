//! Limits and target shared by the direct checker.


/// The machine being compiled for, as `#target` reports it while compiling.
///
/// The compiler knows only these primitive facts. Anything else about the target, such as the
/// width of an index or the size of a type, is meant to be worked out in Gin from them and from
/// the declared ranges of types, so that it is written once, in the language, not in the compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    /// The target triple, as compile-time text: `aarch64-apple-darwin`.
    pub triple: &'static str,
    /// The width of a machine register, in bits.
    pub register_bits: u32,
}

impl Target {
    pub const AARCH64_DARWIN: Self = Self {
        triple: "aarch64-apple-darwin",
        register_bits: 64,
    };
    pub const X86_64_DARWIN: Self = Self {
        triple: "x86_64-apple-darwin",
        register_bits: 64,
    };
    pub const AARCH64_LINUX: Self = Self {
        triple: "aarch64-unknown-linux-gnu",
        register_bits: 64,
    };
    pub const X86_64_LINUX: Self = Self {
        triple: "x86_64-unknown-linux-gnu",
        register_bits: 64,
    };

    /// The machine this compiler is running on, which is what the new path builds for.
    pub const fn host() -> Self {
        if cfg!(all(target_arch = "aarch64", target_os = "macos")) {
            Self::AARCH64_DARWIN
        } else if cfg!(all(target_arch = "x86_64", target_os = "macos")) {
            Self::X86_64_DARWIN
        } else if cfg!(all(target_arch = "aarch64", target_os = "linux")) {
            Self::AARCH64_LINUX
        } else {
            Self::X86_64_LINUX
        }
    }
}

/// Explicit per-check resource limits, and the target a check answers `#target` for. Zero is
/// allowed for a limit and fails closed.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Combined count of type declarations and value definitions.
    pub definitions_max: u32,
    pub parameters_max: u32,
    /// Dependency/expression visits, continuations, and proposition construction/copies per file.
    /// Also independently bounds input normalization visits/record-sized byte-work batches.
    /// Normalization storage bytes are bounded by this count times one declaration plus one pointer.
    pub work_max: u32,
    /// Ownership-flow visits, liveness sets, state snapshots, and joins per file.
    pub flow_work_max: u32,
    /// What `#target` reports. The host unless the caller says otherwise.
    pub target: Target,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            definitions_max: 4096,
            parameters_max: 64,
            work_max: 100_000,
            flow_work_max: 1_000_000,
            target: Target::host(),
        }
    }
}
