use chumsky::span::SimpleSpan;
use std::ops::{Deref, DerefMut};

/// A value paired with its source span.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Spanned<T>(pub T, pub SimpleSpan);

impl<T> Spanned<T> {
    pub fn into_parts(self) -> (T, SimpleSpan) {
        (self.0, self.1)
    }

    /// Map the inner value while preserving the span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned(f(self.0), self.1)
    }
}

impl<T> Deref for Spanned<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Spanned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
