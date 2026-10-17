use std::collections::HashMap;
use std::ops::{Deref, Index};

use indexmap::IndexMap;
use internment::Intern;

use crate::Bind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DefinitionId(pub Intern<String>, pub u32);

impl DefinitionId {
    pub fn named(name: Intern<String>) -> Self {
        Self(name, 0)
    }

    pub fn member(name: Intern<String>, member: u32) -> Self {
        Self(name, member)
    }

    pub fn with_name(self, name: Intern<String>) -> Self {
        Self(name, self.1)
    }
}

impl From<Intern<String>> for DefinitionId {
    fn from(name: Intern<String>) -> Self {
        Self::named(name)
    }
}

pub type OverloadGroups = HashMap<Intern<String>, Vec<DefinitionId>>;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DefMap {
    entries: IndexMap<DefinitionId, Bind>,
    groups: OverloadGroups,
}

impl Deref for DefMap {
    type Target = IndexMap<DefinitionId, Bind>;

    fn deref(&self) -> &Self::Target {
        &self.entries
    }
}

impl DefMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn groups(&self) -> &OverloadGroups {
        &self.groups
    }

    pub fn insert(&mut self, id: impl Into<DefinitionId>, bind: Bind) -> Option<Bind> {
        let id = id.into();
        if !self.entries.contains_key(&id) {
            self.groups.entry(id.0).or_default().push(id);
        }
        self.entries.insert(id, bind)
    }

    pub fn get(&self, id: &(impl Copy + Into<DefinitionId>)) -> Option<&Bind> {
        self.entries.get(&(*id).into())
    }

    pub fn get_mut(&mut self, id: &(impl Copy + Into<DefinitionId>)) -> Option<&mut Bind> {
        self.entries.get_mut(&(*id).into())
    }

    pub fn contains_key(&self, id: &(impl Copy + Into<DefinitionId>)) -> bool {
        self.entries.contains_key(&(*id).into())
    }

    pub fn shift_remove(&mut self, id: &(impl Copy + Into<DefinitionId>)) -> Option<Bind> {
        let id = (*id).into();
        let removed = self.entries.shift_remove(&id)?;
        if let Some(group) = self.groups.get_mut(&id.0) {
            group.retain(|member| *member != id);
            if group.is_empty() {
                self.groups.remove(&id.0);
            }
        }
        Some(removed)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&DefinitionId, &mut Bind) -> bool) {
        self.entries.retain(|id, bind| keep(id, bind));
        self.groups.retain(|_, members| {
            members.retain(|id| self.entries.contains_key(id));
            !members.is_empty()
        });
    }

    pub fn values_mut(&mut self) -> indexmap::map::ValuesMut<'_, DefinitionId, Bind> {
        self.entries.values_mut()
    }

    pub fn iter_mut(&mut self) -> indexmap::map::IterMut<'_, DefinitionId, Bind> {
        self.entries.iter_mut()
    }
}

impl<K: Copy + Into<DefinitionId>> Index<&K> for DefMap {
    type Output = Bind;

    fn index(&self, id: &K) -> &Self::Output {
        &self.entries[&(*id).into()]
    }
}

impl IntoIterator for DefMap {
    type Item = (DefinitionId, Bind);
    type IntoIter = indexmap::map::IntoIter<DefinitionId, Bind>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

impl<'a> IntoIterator for &'a DefMap {
    type Item = (&'a DefinitionId, &'a Bind);
    type IntoIter = indexmap::map::Iter<'a, DefinitionId, Bind>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl<'a> IntoIterator for &'a mut DefMap {
    type Item = (&'a DefinitionId, &'a mut Bind);
    type IntoIter = indexmap::map::IterMut<'a, DefinitionId, Bind>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter_mut()
    }
}
