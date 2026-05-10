use std::collections::HashSet;
use std::ops::ControlFlow;

use ast::visit::{Visitor, walk_bind_value, walk_expr, walk_fn_call};
use ast::{Expr, FnCall, Spanned};
use diagnostic::type_::TypeSymptom;
use diagnostic::DiagnosticLike;
use internment::Intern;

struct RefCollector<'a> {
    refs: &'a mut HashSet<Intern<String>>,
}

impl Visitor for RefCollector<'_> {
    fn visit_fn_call(&mut self, call: &FnCall) -> ControlFlow<()> {
        self.refs.insert(call.path.root);
        walk_fn_call(self, call)
    }
}

pub(crate) fn detect_unused_bindings(
    exprs: &[Spanned<Expr>],
    ret: &Option<Box<Spanned<Expr>>>,
    symptoms: &mut Vec<diagnostic::Diagnostic>,
) {
    let mut suffix_refs: HashSet<Intern<String>> = HashSet::new();
    if let Some(e) = ret.as_ref() {
        let mut collector = RefCollector {
            refs: &mut suffix_refs,
        };
        let _ = walk_expr(&mut collector, e);
    }
    let mut unused_spans: Vec<_> = Vec::new();
    for expr in exprs.iter().rev() {
        if let Expr::Bind(inner) = &**expr {
            let name = inner.name();
            if !suffix_refs.contains(&name) && !name.starts_with('_') {
                unused_spans.push((name, inner.name_span));
            }
            let mut collector = RefCollector {
                refs: &mut suffix_refs,
            };
            let _ = walk_bind_value(&mut collector, inner.value());
        } else {
            let mut collector = RefCollector {
                refs: &mut suffix_refs,
            };
            let _ = walk_expr(&mut collector, expr);
        }
    }
    for (name, span) in unused_spans.into_iter().rev() {
        symptoms.push(
            TypeSymptom::UnusedBinding {
                name: name.to_string(),
            }
            .into_diagnostic(span),
        );
    }
}
