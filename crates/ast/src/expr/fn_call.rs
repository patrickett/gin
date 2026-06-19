use crate::expr::Expr;
use crate::expr::Typed;
use crate::path::ModPath;
use crate::span::Spanned;
use internment::Intern;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnCall {
    pub path: Spanned<ModPath>,
    pub args: Option<Vec<Typed<Expr>>>,
}

impl FnCall {
    /// Flattens a qualified path into a single symbol name for codegen (e.g. `io.print`).
    pub fn mangled_name(&self) -> Intern<String> {
        if self.path.segments.is_empty() {
            self.path.root
        } else {
            let mut joined = self.path.root.as_str().to_string();
            for seg in &self.path.segments {
                joined.push('.');
                joined.push_str(seg.as_str());
            }
            Intern::<String>::new(joined)
        }
    }
}
