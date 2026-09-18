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

pub type ProjectedResultEvidence = HashMap<Vec<usize>, ResultEvidence>;
