use std::fmt;

use internment::Intern;
use itertools::Itertools;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupPath {
    pub root: Intern<String>,
    pub segments: Vec<Intern<String>>,
}

impl GroupPath {
    pub fn new(root: Intern<String>, segments: Vec<Intern<String>>) -> Self {
        Self { root, segments }
    }

    pub fn root(root: Intern<String>) -> Self {
        Self::new(root, Vec::new())
    }

    pub fn is_ancestor_of(&self, other: &Self) -> bool {
        self.root == other.root
            && self.segments.len() < other.segments.len()
            && other.segments.starts_with(&self.segments)
    }
}

impl fmt::Display for GroupPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            self.root.as_str(),
            self.segments.iter().format(".")
        )
    }
}
