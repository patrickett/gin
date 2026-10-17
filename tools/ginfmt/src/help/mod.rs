mod analysis;
mod direct_bind;
mod generic_names;
mod nested_construction;
mod parameter_names;
mod redundant_destructure_bindings;

use diagnostic::code::FormatCode;
use std::ops::ControlFlow;
use std::path::Path;

use ast::folder::walk_typed_expr_children;
use ast::{BindValue, CallSyntax, Expr, FnCall, SpanId, Typed};
use diagnostic::{Category, Diagnostic, DiagnosticCode, Span};
use flask::CompileTarget;
use typecheck::TypedExprKind;

use analysis::Analysis;
pub(crate) use direct_bind::format_value_bodies;

#[derive(Clone, Debug)]
pub struct Help {
    pub diagnostic: Diagnostic,
    pub edits: Vec<Edit>,
}

#[derive(Clone, Debug)]
pub struct Edit {
    pub span: Span,
    pub replacement: String,
}

fn edits(helps: &[Help]) -> Vec<&Edit> {
    let mut edits: Vec<_> = helps.iter().flat_map(|help| &help.edits).collect();
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.span.start()));
    edits
}

pub fn suggestions(path: &Path, target: CompileTarget) -> Result<Vec<Help>, String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let source = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    Ok(Analysis::load(&path, source, target)?.suggestions())
}

pub fn fix(path: &Path, target: CompileTarget) -> Result<usize, String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let original = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
    let mut analysis = Analysis::load(&path, original.clone(), target.clone())?;
    let mut count = 0;
    for _ in 0..32 {
        let selected = non_overlapping(analysis.suggestions());
        if selected.is_empty() {
            if count > 0 {
                let current = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
                if current != original {
                    return Err("file changed while computing fixes; no changes written".to_owned());
                }
                std::fs::write(&path, &analysis.parsed.source)
                    .map_err(|error| error.to_string())?;
            }
            return Ok(count);
        }
        let mut source = analysis.parsed.source.clone();
        for edit in edits(&selected) {
            source.replace_range(edit.span.to_range(), &edit.replacement);
        }
        let next = Analysis::load(&path, source, target.clone())?;
        let previous_flaws = analysis.flaws();
        if next
            .flaws()
            .iter()
            .any(|(flaw, count)| *count > previous_flaws.get(flaw).copied().unwrap_or_default())
            || next.calls() != analysis.calls()
        {
            return Err(
                "suggested fixes changed call resolution or introduced errors; no changes written"
                    .to_owned(),
            );
        }
        count += selected.len();
        analysis = next;
    }
    Err("help did not converge; no changes written".to_owned())
}

pub struct HelpContext<'a> {
    pub source: &'a str,
    pub parsed: &'a parser::query::ParseOutput,
    pub typed: &'a typecheck::TypedFileAst,
    pub package: &'a [&'a typecheck::TypedFileAst],
}

pub fn non_overlapping(mut helps: Vec<Help>) -> Vec<Help> {
    helps.retain(|help| !help.edits.is_empty());
    helps.sort_by_key(|help| {
        (
            std::cmp::Reverse(help.diagnostic.span.len()),
            help.diagnostic.span.start(),
        )
    });
    let mut selected: Vec<Help> = Vec::new();
    for help in helps {
        if selected.iter().all(|selected| {
            selected.edits.iter().all(|left| {
                help.edits
                    .iter()
                    .all(|right| !overlaps(left.span, right.span))
            })
        }) {
            selected.push(help);
        }
    }
    selected
}

impl Analysis {
    fn suggestions(&self) -> Vec<Help> {
        let package: Vec<_> = self
            .output
            .files
            .iter()
            .filter_map(|file| file.typed.as_ref())
            .collect();
        HelpContext {
            source: &self.parsed.source,
            parsed: &self.parsed.output,
            typed: self.typed(),
            package: &package,
        }
        .suggestions()
    }
}

impl HelpContext<'_> {
    pub fn suggestions(&self) -> Vec<Help> {
        if self
            .parsed
            .symptoms
            .iter()
            .any(|symptom| symptom.category == Category::Flaw)
        {
            return Vec::new();
        }
        let mut helps = self.parameter_name_suggestions();
        helps.extend(self.generic_name_suggestions());
        helps.extend(self.direct_bind_suggestions());
        helps.extend(self.nested_construction_suggestions());
        helps.extend(self.redundant_destructure_binding_suggestions());
        for bind in self.parsed.ast.defs.values() {
            match &bind.value {
                BindValue::Expr(expr) => self.visit(&expr.value, expr.span_id, &mut helps),
                BindValue::Body { exprs, ret } => {
                    for expr in exprs.iter().chain(&ret.values) {
                        self.visit(&expr.value, expr.span_id, &mut helps);
                    }
                }
                BindValue::Unassigned => {}
            }
        }
        for (expr, span) in &self.parsed.ast.exprs {
            let first = helps.len();
            self.visit(expr, *span, &mut helps);
            let span = self.parsed.ast.span_table.get(*span);
            for help in &mut helps[first..] {
                for edit in &mut help.edits {
                    if edit.span == span {
                        edit.replacement = format!("({})", edit.replacement);
                        help.diagnostic.help = Some(format!("use `{}`", edit.replacement));
                    }
                }
            }
        }
        helps.sort_by_key(|help| (help.diagnostic.span.start(), help.diagnostic.span.end()));
        helps.dedup_by(|left, right| left.diagnostic.span == right.diagnostic.span);
        helps
    }

    fn visit(&self, expr: &Expr, span: SpanId, helps: &mut Vec<Help>) {
        match expr {
            Expr::FnCall(call) => {
                if let Some(replacement) = self.receiver_call(span, call) {
                    // fmt-receiver-call
                    self.suggest(
                        expr,
                        span,
                        DiagnosticCode::Format(FormatCode::ReceiverCall),
                        "call this method through its receiver",
                        replacement,
                        helps,
                    );
                }
            }
            Expr::TagCall(call) => {
                for arg in &call.args {
                    if let Expr::Bind(bind) = &arg.value
                        && bind.params.is_none()
                        && let BindValue::Expr(value) = &bind.value
                        && let Expr::FnCall(value) = &value.value
                        && value.args.is_none()
                        && value.path.segments.is_empty()
                        && value.path.root == bind.name
                    {
                        // fmt-field-shorthand
                        self.suggest(
                            &arg.value,
                            arg.span_id,
                            DiagnosticCode::Format(FormatCode::FieldShorthand),
                            "omit the repeated field name",
                            bind.name.to_string(),
                            helps,
                        );
                    }
                }
            }
            _ => {}
        }
        let _ = walk_typed_expr_children(expr, &mut |child| {
            self.visit(&child.value, child.span_id, helps);
            ControlFlow::Continue(())
        });
    }

    fn suggest(
        &self,
        expr: &Expr,
        span: SpanId,
        code: DiagnosticCode,
        message: &str,
        replacement: String,
        helps: &mut Vec<Help>,
    ) {
        let mut span = self.parsed.ast.span_table.get(span);
        if let Expr::Bind(bind) = expr {
            span = span.merge(self.parsed.ast.span_table.get(bind.name_span));
        }
        let Some(text) = self.source.get(span.to_range()) else {
            return;
        };
        let span = Span::from_range(span.start()..span.start() + text.trim_end().len());
        if text.trim_end() == replacement
            || span.is_empty()
            || self
                .parsed
                .comments
                .iter()
                .any(|comment| overlaps(span, self.parsed.ast.span_table.get(comment.span)))
        {
            return;
        }
        helps.push(Help {
            diagnostic: Diagnostic::new(code, message)
                .with_category(Category::Help)
                .with_help(format!("use `{replacement}`"))
                .at_span(span),
            edits: vec![Edit { span, replacement }],
        });
    }

    fn receiver_call(&self, span: SpanId, call: &FnCall) -> Option<String> {
        if call.syntax != CallSyntax::Path || call.path.segments.len() != 1 {
            return None;
        }
        let (receiver, args) = call.args.as_deref()?.split_first()?;
        let method = call.path.segments.last()?;
        let typed = self.typed;
        let span = self.parsed.ast.span_table.get(span);
        if typed
            .resolved_member_calls
            .keys()
            .any(|id| typed.span_table.get(typed.exprs.span[id.as_usize()]) == span)
        {
            return None;
        }
        let mut targets = typed
            .exprs
            .kind
            .iter()
            .enumerate()
            .filter_map(|(index, kind)| {
                if typed.span_table.get(typed.exprs.span[index]) == span
                    && let TypedExprKind::FnCall {
                        target,
                        args: Some(_),
                        ..
                    } = kind
                {
                    Some(*target)
                } else {
                    None
                }
            });
        let target = targets.next().or_else(|| self.declared_target(span))?;
        if targets.any(|other| other != target) {
            return None;
        }
        let definition = self
            .package
            .iter()
            .find_map(|file| file.defs.get(&target))?;
        if definition.params.first()?.0.as_str() != "self" || definition.receiver_type.is_none() {
            return None;
        }
        let receiver_type = &definition.params.first()?.1;
        if definition
            .receiver_type
            .as_ref()?
            .named_instance()?
            .declaration
            != receiver_type
                .named_instance_stripping_reference_wrappers()?
                .declaration
            || self
                .package
                .iter()
                .flat_map(|file| file.defs.iter())
                .any(|(id, candidate)| {
                    id.0 == target.0
                        && candidate.param_conventions.first()
                            != definition.param_conventions.first()
                })
        {
            return None;
        }
        if typed
            .type_registry
            .resolved_definition_for_type(receiver_type.without_reference_wrappers())
            .named_field(*method)
            .is_some()
        {
            return None;
        }
        if matches!(receiver.value, Expr::Ref { .. } | Expr::Bind(_)) {
            return None;
        }
        let receiver_text = self.render(receiver)?;
        let receiver_text = if matches!(
            receiver.value,
            Expr::SelfRef
                | Expr::FnCall(_)
                | Expr::RecordGet { .. }
                | Expr::BufGet { .. }
                | Expr::TagCall(_)
        ) {
            receiver_text.to_owned()
        } else {
            format!("({receiver_text})")
        };
        if args.is_empty() {
            Some(format!("{receiver_text}.{method}"))
        } else {
            let args: Option<Vec<_>> = args.iter().map(|arg| self.render(arg)).collect();
            Some(format!("{receiver_text}.{method}({})", args?.join(", ")))
        }
    }

    fn declared_target(&self, span: Span) -> Option<typecheck::DefId> {
        let ast = &self.typed.eval_ast;
        let mut target = None;
        for id in self.typed.defs.keys() {
            let Some(bind) = ast.defs.get(id) else {
                continue;
            };
            let mut inspect =
                |expr: &Typed<Expr>| declared_call(expr, span, &self.typed.span_table, &mut target);
            match &bind.value {
                BindValue::Expr(expr) => {
                    let _ = inspect(expr);
                }
                BindValue::Body { exprs, ret } => {
                    for expr in exprs.iter().chain(&ret.values) {
                        if inspect(expr).is_break() {
                            break;
                        }
                    }
                }
                BindValue::Unassigned => {}
            }
            if target.is_some() {
                break;
            }
        }
        let target = target?;
        let mut candidates = self
            .package
            .iter()
            .flat_map(|file| file.defs.keys())
            .filter(|id| id.0 == target.0);
        let candidate = *candidates.next()?;
        candidates.next().is_none().then_some(candidate)
    }

    fn render(&self, expr: &Typed<Expr>) -> Option<String> {
        let span = self.parsed.ast.span_table.get(expr.span_id);
        let mut text = self.source.get(span.to_range())?.to_owned();
        let mut helps = Vec::new();
        self.visit(&expr.value, expr.span_id, &mut helps);
        let helps = non_overlapping(helps);
        for edit in edits(&helps) {
            if edit.span.start() < span.start() || edit.span.end() > span.end() {
                return None;
            }
            text.replace_range(
                edit.span.start() - span.start()..edit.span.end() - span.start(),
                &edit.replacement,
            );
        }
        Some(text.trim().to_owned())
    }
}

fn overlaps(left: Span, right: Span) -> bool {
    left.start() < right.end() && right.start() < left.end()
}

fn declared_call(
    expr: &Typed<Expr>,
    span: Span,
    spans: &ast::SpanTable,
    target: &mut Option<typecheck::DefId>,
) -> ControlFlow<()> {
    if spans.get(expr.span_id) == span
        && let Expr::FnCall(call) = &expr.value
    {
        *target = Some(
            call.definition
                .unwrap_or_else(|| typecheck::DefId::named(call.mangled_name())),
        );
        return ControlFlow::Break(());
    }
    walk_typed_expr_children(&expr.value, &mut |child| {
        declared_call(child, span, spans, target)
    })
}
