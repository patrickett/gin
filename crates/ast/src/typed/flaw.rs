//! Flaw types — structured diagnostic information attached to AST nodes.
//!
//! Two domains: [`ParseFlaw`] for `ParseAst` nodes and [`TypeFlaw`] for `TypedFileAst` nodes.

/// Covers lex errors, parse errors, and import-structural errors.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParseFlaw {
    // --- Lex errors ---
    UnclosedString,
    InvalidInteger,
    InvalidFloat,
    OverflowIndent,
    UnexpectedCharacter,

    // --- Parse errors ---
    UnexpectedToken,
    Custom(String),
    EmptyParens {
        suggested: String,
    },
    UnusedValue {
        value: String,
    },
    DirectFileImport {
        path: String,
    },

    // --- Import structural flaws (pre-resolution) ---
    ImportConflict {
        path: String,
        qualifier_a: String,
        qualifier_b: String,
    },
    ImportTargetNotFound {
        path: String,
    },
    ImportLocalMustEndInGin {
        path: String,
    },
    ImportLocalNotFound {
        path: String,
    },
    ImportFolderMissingConfig {
        folder: String,
    },
    ImportMissingExport {
        folder: String,
        export: String,
    },
    ImportExportTargetNotFound {
        export: String,
        folder: String,
        path: String,
    },
    ImportAmbiguousLocalRoot {
        name: String,
        file_path: String,
        folder_path: String,
    },
    ImportFileHasSegments {
        file_path: String,
        segment: String,
    },
    ImportUnknownDependency {
        name: String,
    },
    ImportDependencyMissingConfig {
        name: String,
        path: String,
    },
    ImportMissingConfig {
        dir: String,
    },
    ImportChainedExportNotFolder {
        path: String,
    },
    ImportCycle {
        chain: String,
    },
    ImportLocalFolderRequiresAs {
        path: String,
    },
    ImportNestedPackageNotFound {
        parent: String,
        segment: String,
    },
    ImportPackageHasNoGinFiles {
        dir: String,
    },
    ImportDuplicateTopLevel {
        symbol: String,
    },
    ImportNotExported {
        symbol: String,
        module: String,
    },
}

/// One domain — all type, flow, and resolved-reference flaws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeFlaw {
    // --- Name resolution ---
    UnknownBinding {
        name: String,
        did_you_mean: Option<String>,
    },
    UnknownTag {
        name: String,
    },
    NotExpr {
        name: String,
    },
    NotAVariant {
        name: String,
        union_name: String,
    },
    SelfOutsideMethod,

    // --- Arity / generics ---
    ArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },
    UnresolvedTypeParam {
        name: String,
    },
    ConstraintViolation {
        param: String,
        expected: String,
        got: String,
    },
    PositionalAfterDefault {
        name: String,
    },

    // --- Type mismatch ---
    Mismatch,
    InferenceFailed,

    // --- Control flow ---
    MissingElseArm,
    ConditionNotBool {
        got: String,
    },
    EmptyReturn {
        expected_type: String,
    },

    // --- Ownership / flow ---
    UseOfMovedValue {
        name: String,
    },
    LinValueNotConsumed {
        name: String,
    },
    CannotPassReadonlyAsMut {
        name: String,
    },

    // --- Bounds ---
    IndexOutOfBounds {
        index: i128,
        size: usize,
    },
    ImpossibleCheck {
        reason: String,
    },

    // --- Lint-level ---
    UnusedBinding {
        name: String,
    },
}
