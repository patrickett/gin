use crate::{ImportEntry, ImportMember, ImportSource};

use super::{Kind, Lookup};

impl<'syntax> Lookup<'syntax> {
    pub(super) fn import_entry_children(&mut self, index: usize, entry: &'syntax ImportEntry) {
        self.add(
            index,
            Kind::ImportSource(&entry.source),
            entry.source.span_id,
        );
        for member in &entry.members {
            self.add(index, Kind::ImportMember(member), member.span_id);
        }
        if let Some(alias) = &entry.alias {
            self.name(index, alias);
        }
    }

    pub(super) fn import_source_children(&mut self, index: usize, source: &'syntax ImportSource) {
        match source {
            ImportSource::Module(path) => {
                for name in path {
                    self.name(index, name);
                }
            }
            ImportSource::Current(name) => self.name(index, name),
            ImportSource::Path => {}
        }
    }

    pub(super) fn import_member_children(&mut self, index: usize, member: &'syntax ImportMember) {
        for name in &member.path {
            self.name(index, name);
        }
        if let Some(alias) = &member.alias {
            self.name(index, alias);
        }
    }
}
