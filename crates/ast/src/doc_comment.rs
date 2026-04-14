// TODO: Implement doc comment lexer and parser support
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocComment(pub String);

impl DocComment {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
