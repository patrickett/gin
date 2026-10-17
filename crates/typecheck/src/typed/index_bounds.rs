use super::ExprId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SequenceIndexCheck {
    Runtime,
    Proven { buffer: ExprId, index: ExprId },
}

impl SequenceIndexCheck {
    pub(crate) fn proves(self, buffer: ExprId, index: ExprId) -> bool {
        self == Self::Proven { buffer, index }
    }
}
