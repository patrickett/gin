use crate::Fingerprint;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetProfile {
    pub target: String,
}

impl TargetProfile {
    pub(crate) fn sort_key(&self) -> &str {
        &self.target
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetRealization {
    pub target: String,
    pub semantic_surface_fingerprint: Fingerprint,
    pub public_closure_fingerprint: Fingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TargetRealizationExpectation {
    pub target: String,
    pub semantic_surface_fingerprint: Fingerprint,
    pub public_closure_fingerprint: Fingerprint,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CachedTargetRealization {
    Decoded(TargetRealization),
    Corrupt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetRealizationRecovery {
    Loaded(TargetRealization),
    Regenerated {
        realization: TargetRealization,
        code: &'static str,
    },
    Unavailable {
        code: &'static str,
    },
}

pub fn recover_target_realization(
    cached: Option<CachedTargetRealization>,
    expected: &TargetRealizationExpectation,
    regenerated: Option<TargetRealization>,
) -> TargetRealizationRecovery {
    let failure = match cached {
        None => "interface-target-realization-missing",
        Some(CachedTargetRealization::Corrupt) => "interface-target-realization-corrupt",
        Some(CachedTargetRealization::Decoded(realization))
            if realization.target != expected.target =>
        {
            "interface-target-realization-mismatch"
        }
        Some(CachedTargetRealization::Decoded(realization))
            if realization.semantic_surface_fingerprint
                != expected.semantic_surface_fingerprint
                || realization.public_closure_fingerprint
                    != expected.public_closure_fingerprint =>
        {
            "interface-target-realization-stale"
        }
        Some(CachedTargetRealization::Decoded(realization)) => {
            return TargetRealizationRecovery::Loaded(realization);
        }
    };
    regenerated.map_or(
        TargetRealizationRecovery::Unavailable { code: failure },
        |realization| TargetRealizationRecovery::Regenerated {
            realization,
            code: failure,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expectation() -> TargetRealizationExpectation {
        TargetRealizationExpectation {
            target: "x86_64-unknown-linux".into(),
            semantic_surface_fingerprint: Fingerprint::from_bytes(b"surface"),
            public_closure_fingerprint: Fingerprint::from_bytes(b"closure"),
        }
    }

    fn matching() -> TargetRealization {
        let expected = expectation();
        TargetRealization {
            target: expected.target,
            semantic_surface_fingerprint: expected.semantic_surface_fingerprint,
            public_closure_fingerprint: expected.public_closure_fingerprint,
        }
    }

    #[test]
    fn missing_stale_corrupt_and_wrong_target_realizations_regenerate() {
        let expected = expectation();
        let regenerated = matching();
        let cases = [
            (None, "interface-target-realization-missing"),
            (
                Some(CachedTargetRealization::Corrupt),
                "interface-target-realization-corrupt",
            ),
            (
                Some(CachedTargetRealization::Decoded(TargetRealization {
                    target: "arm64-unknown-linux".into(),
                    ..matching()
                })),
                "interface-target-realization-mismatch",
            ),
            (
                Some(CachedTargetRealization::Decoded(TargetRealization {
                    semantic_surface_fingerprint: Fingerprint::from_bytes(b"old"),
                    ..matching()
                })),
                "interface-target-realization-stale",
            ),
        ];

        for (cached, code) in cases {
            assert_eq!(
                recover_target_realization(cached, &expected, Some(regenerated.clone())),
                TargetRealizationRecovery::Regenerated {
                    realization: regenerated.clone(),
                    code,
                }
            );
        }
    }

    #[test]
    fn unavailable_realization_reports_mismatch_without_mutating_the_base() {
        let expected = expectation();
        assert_eq!(
            recover_target_realization(
                Some(CachedTargetRealization::Decoded(TargetRealization {
                    target: "wasm32-unknown-unknown".into(),
                    ..matching()
                })),
                &expected,
                None,
            ),
            TargetRealizationRecovery::Unavailable {
                code: "interface-target-realization-mismatch"
            }
        );
        assert_eq!(
            expected.semantic_surface_fingerprint,
            Fingerprint::from_bytes(b"surface")
        );
    }
}
