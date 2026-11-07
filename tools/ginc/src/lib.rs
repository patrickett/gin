mod build_new;
mod build_output;
mod flaw;
mod pipeline;
pub use flaw::{enclosing_span, fact_claim, failure_diagnostic, failure_message, note_diagnostic};
/// Shared checker diagnostics for the compiler and interactive tools.
pub use pipeline::{failure_details, note_code};
pub mod cli;
pub mod compile;
mod lower_next;
pub use lower_next::{
    InstanceLayout, MemberLayout, RegisterLayout, SumLayout, instance_layouts, parameter_layout,
};
mod mono;
pub mod reporting;

#[cfg(test)]
mod cutover_tests;

/// The MLIR text of each capsule as the compiler lowers a package's modules (`root` is the
/// program, the others libraries), for tests that make capsules another way, such as reading
/// them back from interface artifacts.
#[doc(hidden)]
pub fn lower_capsules(
    capsules: &[&typecheck::CheckedSourceModule],
    root: usize,
) -> Result<Vec<(String, String)>, String> {
    build_new::lower_capsules(capsules, Some(root)).map_err(|error| error.to_string())
}
