use crate::expr::Expr;
use crate::expr::Typed;
use crate::path::ModPath;
use crate::span::Spanned;
use internment::Intern;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallSyntax {
    Path,
    Receiver,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FnCall {
    pub syntax: CallSyntax,
    pub definition: Option<crate::DefinitionId>,
    pub path: Spanned<ModPath>,
    pub args: Option<Vec<Typed<Expr>>>,
}

impl FnCall {
    pub fn with_path_receiver(&self) -> Option<Self> {
        let (member, fields) = self.path.segments.split_last()?;
        let mut receiver = Typed::infer(
            Expr::FnCall(Self {
                syntax: CallSyntax::Path,
                definition: None,
                path: Spanned::new(ModPath::new(self.path.root, vec![]), self.path.root_span),
                args: None,
            }),
            self.path.root_span,
        );
        for (index, field) in fields.iter().enumerate() {
            receiver = Typed::infer(
                Expr::RecordGet {
                    base: Box::new(receiver),
                    field: *field,
                },
                self.path
                    .segment_spans
                    .get(index)
                    .copied()
                    .unwrap_or(self.path.span_id),
            );
        }
        let mut args = vec![receiver];
        args.extend(self.args.clone().unwrap_or_default());
        Some(Self {
            syntax: CallSyntax::Receiver,
            definition: None,
            path: Spanned::new(ModPath::new(*member, vec![]), self.path.span_id),
            args: Some(args),
        })
    }

    /// Flattens a qualified path into a single symbol name for codegen (e.g. `io.print`).
    pub fn mangled_name(&self) -> Intern<String> {
        if self.path.segments.is_empty() {
            self.path.root
        } else {
            Intern::<String>::new(crate::path::join_dotted(
                self.path.root,
                &self.path.segments,
            ))
        }
    }
}
