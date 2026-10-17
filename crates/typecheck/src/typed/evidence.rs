use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MathematicalComparison {
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Equal,
    NotEqual,
}

impl MathematicalComparison {
    pub fn complement(self) -> Self {
        match self {
            Self::Less => Self::GreaterOrEqual,
            Self::LessOrEqual => Self::Greater,
            Self::Greater => Self::LessOrEqual,
            Self::GreaterOrEqual => Self::Less,
            Self::Equal => Self::NotEqual,
            Self::NotEqual => Self::Equal,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValueEvidence {
    Proposition(EvidenceProposition),
    Alternatives(ResultEvidence),
}

impl ValueEvidence {
    pub(crate) fn map_operands(
        &self,
        mut map: impl FnMut(ExprId) -> Option<ExprId>,
    ) -> Option<Self> {
        let mut proposition = |proposition: &EvidenceProposition| {
            let EvidenceProposition::IntegerComparison {
                lhs,
                rhs,
                comparison,
            } = proposition;
            Some(EvidenceProposition::IntegerComparison {
                lhs: map(*lhs)?,
                rhs: map(*rhs)?,
                comparison: *comparison,
            })
        };
        Some(match self {
            Self::Proposition(evidence) => Self::Proposition(proposition(evidence)?),
            Self::Alternatives(evidence) => Self::Alternatives(ResultEvidence {
                owner: evidence.owner.clone(),
                alternatives: evidence
                    .alternatives
                    .iter()
                    .map(|alternative| {
                        Some(AlternativeEvidence {
                            label: alternative.label,
                            proposition: proposition(&alternative.proposition)?,
                        })
                    })
                    .collect::<Option<Vec<_>>>()?,
            }),
        })
    }
}

impl From<MathematicalComparison> for ast::ProofRelation {
    fn from(value: MathematicalComparison) -> Self {
        match value {
            MathematicalComparison::Less => Self::Less,
            MathematicalComparison::LessOrEqual => Self::LessOrEqual,
            MathematicalComparison::Greater => Self::Greater,
            MathematicalComparison::GreaterOrEqual => Self::GreaterOrEqual,
            MathematicalComparison::Equal => Self::Equal,
            MathematicalComparison::NotEqual => Self::NotEqual,
        }
    }
}

impl From<ast::ProofRelation> for MathematicalComparison {
    fn from(value: ast::ProofRelation) -> Self {
        match value {
            ast::ProofRelation::Less => Self::Less,
            ast::ProofRelation::LessOrEqual => Self::LessOrEqual,
            ast::ProofRelation::Greater => Self::Greater,
            ast::ProofRelation::GreaterOrEqual => Self::GreaterOrEqual,
            ast::ProofRelation::Equal => Self::Equal,
            ast::ProofRelation::NotEqual => Self::NotEqual,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EvidenceProposition {
    IntegerComparison {
        lhs: ExprId,
        rhs: ExprId,
        comparison: MathematicalComparison,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AlternativeEvidence {
    pub label: Intern<String>,
    pub proposition: EvidenceProposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResultEvidence {
    pub owner: ast::ResultFamilyOwner,
    pub alternatives: Vec<AlternativeEvidence>,
}

pub type ProjectedResultEvidence = HashMap<Vec<usize>, ValueEvidence>;
