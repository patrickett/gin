use crate::prelude::*;
use diagnostic::DiagnosticCode;
use diagnostic::code::CodegenCode;

pub(super) enum PlaceMergeKind {
    Loop,
    Branch,
}

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn incoming_place_bindings(
        &self,
        versions: &[typecheck::PlaceVersionId],
        symtab: &ScopedSymbolTable<'c>,
        kind: PlaceMergeKind,
    ) -> Option<Vec<(internment::Intern<String>, Ty, Value<'c, 'c>)>> {
        let (context, invalid_code, missing_code) = match kind {
            PlaceMergeKind::Loop => (
                "loop",
                // codegen-loop-phi-invalid
                DiagnosticCode::Codegen(CodegenCode::LoopPhiInvalid),
                // codegen-loop-binding-missing
                DiagnosticCode::Codegen(CodegenCode::LoopBindingMissing),
            ),
            PlaceMergeKind::Branch => (
                "branch",
                // codegen-branch-join-invalid
                DiagnosticCode::Codegen(CodegenCode::BranchJoinInvalid),
                // codegen-branch-binding-missing
                DiagnosticCode::Codegen(CodegenCode::BranchBindingMissing),
            ),
        };
        let mut bindings = Vec::with_capacity(versions.len());
        for id in versions {
            let Some(version) = self.program.place_versions.get(id.0 as usize) else {
                self.emit_symptom(
                    invalid_code,
                    format!("{context} references missing place version {}", id.0),
                );
                return None;
            };
            let Some(place) = self.program.places.get(version.place.0 as usize) else {
                self.emit_symptom(
                    invalid_code,
                    format!(
                        "{context} place version {} references missing place {}",
                        id.0, version.place.0
                    ),
                );
                return None;
            };
            let Some(binding) = symtab.get(&place.name) else {
                self.emit_symptom(
                    missing_code,
                    format!(
                        "place `{}` has no incoming binding for this {context}",
                        place.name
                    ),
                );
                return None;
            };
            if !binding.is_slot {
                bindings.push((place.name, place.ty.clone(), binding.value));
            }
        }
        Some(bindings)
    }
}
