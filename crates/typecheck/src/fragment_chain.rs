use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fragment<T> {
    Owned(T),
    Shared(Arc<T>),
}

impl<T> Fragment<T> {
    pub fn as_ref(&self) -> &T {
        match self {
            Self::Owned(value) => value,
            Self::Shared(value) => value,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FragmentChain<T> {
    fragments: Vec<Fragment<T>>,
}

impl<T> FragmentChain<T> {
    pub fn from_owned<I>(values: I) -> Self
    where
        I: IntoIterator<Item = T>,
    {
        Self {
            fragments: values.into_iter().map(Fragment::Owned).collect(),
        }
    }

    pub fn from_shared<I>(values: I) -> Self
    where
        I: IntoIterator<Item = Arc<T>>,
    {
        Self {
            fragments: values.into_iter().map(Fragment::Shared).collect(),
        }
    }

    pub fn new() -> Self {
        Self {
            fragments: Vec::new(),
        }
    }

    pub fn push_owned(&mut self, value: T) {
        self.fragments.push(Fragment::Owned(value));
    }

    pub fn push_shared(&mut self, value: Arc<T>) {
        self.fragments.push(Fragment::Shared(value));
    }

    pub fn append(&mut self, other: Self) {
        self.fragments.extend(other.fragments);
    }

    pub fn iter_fragments(&self) -> impl Iterator<Item = &Fragment<T>> {
        self.fragments.iter()
    }

    pub fn len(&self) -> usize {
        self.fragments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fragments.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.fragments.iter().map(Fragment::as_ref)
    }

    pub fn into_fragments(self) -> impl Iterator<Item = Fragment<T>> {
        self.fragments.into_iter()
    }
}

impl<T: Clone> FragmentChain<T> {
    pub fn materialize(self) -> Vec<T> {
        self.fragments
            .into_iter()
            .map(|fragment| match fragment {
                Fragment::Owned(value) => value,
                Fragment::Shared(value) => (*value).clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_and_shared_fragments_preserve_order() {
        let shared = Arc::new(String::from("shared"));
        let mut chain = FragmentChain::from_owned([String::from("owned")]);
        chain.push_shared(Arc::clone(&shared));
        let tail = FragmentChain::from_owned([String::from(" tail")]);
        chain.append(tail);
        assert_eq!(
            chain.iter().map(String::as_str).collect::<Vec<_>>(),
            ["owned", "shared", " tail"]
        );
        assert_eq!(chain.materialize(), ["owned", "shared", " tail"]);
        assert_eq!(Arc::strong_count(&shared), 1);
    }
}
