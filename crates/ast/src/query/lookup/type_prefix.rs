use crate::{GroupDelimiter, GroupPath, ParameterConvention, PrefixedType};

use super::{Kind, Lookup};

impl<'syntax> Lookup<'syntax> {
    pub(super) fn prefix_children(&mut self, index: usize, prefix: &'syntax PrefixedType) {
        self.add(
            index,
            Kind::TypePrefixKeyword(&prefix.keyword),
            prefix.keyword.span_id,
        );
        if let Some(group) = &prefix.group {
            self.add(index, Kind::GroupPath(group), group.span_id);
        }
        self.expression(index, &prefix.target);
    }

    pub(super) fn convention_children(
        &mut self,
        index: usize,
        convention: &'syntax ParameterConvention,
    ) {
        let keyword = &convention.keyword;
        self.add(index, Kind::Convention(keyword), keyword.span_id);
        if let Some(group) = &convention.group {
            self.add(index, Kind::GroupPath(group), group.span_id);
        }
    }

    pub(super) fn group_children(&mut self, index: usize, group: &'syntax GroupPath) {
        self.add(
            index,
            Kind::GroupDelimiter(GroupDelimiter::Open),
            group.open,
        );
        for component in &group.components {
            self.name(index, component);
        }
        if let Some(close) = group.close {
            self.add(index, Kind::GroupDelimiter(GroupDelimiter::Close), close);
        }
    }
}
