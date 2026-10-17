use std::iter;

use crate::ResolvedProgram;

#[derive(Clone, Copy)]
pub struct EmissionContext<'a> {
    pub(crate) program: ResolvedProgram<'a>,
    pub(crate) related: &'a [ResolvedProgram<'a>],
}

impl<'a> EmissionContext<'a> {
    pub(crate) fn new(program: ResolvedProgram<'a>, related: &'a [ResolvedProgram<'a>]) -> Self {
        Self { program, related }
    }

    pub(crate) fn programs(self) -> impl Iterator<Item = ResolvedProgram<'a>> {
        iter::once(self.program).chain(
            self.related
                .iter()
                .copied()
                .filter(move |program| program.file_id != self.program.file_id),
        )
    }
}
