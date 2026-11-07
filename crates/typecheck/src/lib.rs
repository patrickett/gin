//! Checks Gin source modules and records typed bodies, ownership effects and call summaries.
//!
//! [`check_file`] checks a source file; [`check_module`] checks a module with dependencies.

mod model;
pub mod run;
pub mod source_check;
pub use source_check::regions;
pub use source_check::layout_queries::{arithmetic_type, integer_node, layout_in_file};

pub use model::{Limits, Target};
pub use source_check::{
    AggregateKind, BodyResolver, Discriminant, DomainClaim, BorrowKind, Borrowing, Capability, CheckedArithmetic, CheckedSourceAggregate,
    CheckedSourceBody, CheckedSourceBodyExpr, CheckedSourceBodyKind, CheckedSourceCallee,
    CheckedSourceField, CheckedSourceFile, CheckedSourceFunction, CheckedSourceInstance,
    CheckedSourceInstanceHead, CheckedSourceLoopCarry, CheckedSourceMatchArm, CheckedSourceModule, Evaluated,
    MatchRegion, SelectRegion,
    InfixOperator, SourceCheckError, SourceCheckFailure, SourceCheckNote, SourceCheckNoteKind, SourceFact, BodyDecline, DeclineKind,
    MemoryOp, SourceType, apply, check_file, check_file_diagnostic, check_module, check_module_diagnostic,
    check_module_files, check_module_files_diagnostic, evaluate_body, evaluate_value, evaluate_value_with, FOLD_DEPTH, compare, exact_binary, Exact, interval,
    wrap_into,
    ArtifactError, ArtifactIdentity, ArtifactReader, Classification, Compatibility, ItemChange, artifact_tag_name,
    classify_artifacts, classify_targets, dump_artifact, write_artifact,
};
pub use interface_codec::{Expectation as ArtifactExpectation, ModuleId};
