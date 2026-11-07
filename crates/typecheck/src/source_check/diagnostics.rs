//! Coarse first-failure contexts, attached only at semantic phase boundaries.
use super::*;

pub(super) type DiagnosticResult<T> = std::result::Result<T, SourceCheckFailure>;

/// The unchanged checker error, optionally located in the current source snapshot.
///
/// A span identifies an enclosing declaration, schema body, or import being checked,
/// not necessarily the precise culprit. This is first-error context, not production
/// diagnostic parity or multi-error checking. Global failures (including budgets)
/// and failures without an established valid source context have no span.
///
/// No source text, AST, or dependency source is retained in this value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheckFailure {
    pub error: SourceCheckError,
    pub span: Option<Span>,
    /// Index into the checked file list; present exactly when `span` is.
    pub file: Option<usize>,
    /// What the error was about, naming the culprit, when the checker knows it.
    pub detail: Option<String>,
    /// The exact expression being checked when the error arose, when it lies inside `span`.
    /// Unlike `span` this is not a declaration but the innermost expression last visited.
    pub culprit: Option<Span>,
    /// What the checker had proved about the names in scope at the failure.
    pub facts: Vec<SourceFact>,
}

/// One proved fact about a name in scope, as the checker held it at a failure or a hole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceFact {
    /// `lower <= name <= upper`, the interval the checker proved.
    Interval { name: String, lower: I256, upper: I256 },
    /// `name` is exactly this value.
    Known { name: String, value: I256 },
    /// `name` is a multiple of `modulus`.
    Multiple { name: String, modulus: I256 },
    /// `name = base + offset`: the same number as `base`, offset by a constant.
    Offset { name: String, base: String, offset: I256 },
    /// `left <operator> right`, a relation between two names (`<`, `<=`, `>`, `>=`, `=`).
    Relation { left: String, operator: &'static str, right: String },
}

impl From<SourceCheckError> for SourceCheckFailure {
    fn from(error: SourceCheckError) -> Self {
        Self {
            error,
            span: None,
            file: None,
            detail: None,
            culprit: None,
            facts: Vec::new(),
        }
    }
}

/// A condition worth telling the author about that does not stop the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCheckNoteKind {
    /// A method that only drops its receiver, on a type that may simply be discarded.
    RedundantTerminal,
    /// A loop body or a branch of a conditional that can never run, given what is known.
    UnreachableCode,
    /// An explicit `#todo` hole: the function is accepted and runs until the hole is reached.
    Hole,
}

/// A non-fatal finding, located the same coarse way a failure is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCheckNote {
    pub kind: SourceCheckNoteKind,
    pub span: Option<Span>,
    /// Index into the checked file list; present exactly when `span` is.
    pub file: Option<usize>,
    pub detail: String,
    /// What the checker had proved about the names in scope here (recorded for holes).
    pub facts: Vec<SourceFact>,
}

/// Check a standalone source snapshot with optional first-failure context.
///
/// Acceptance, first-error ordering, and work limits are identical to [`check_file`].
/// Locations are coarse semantic contexts; see [`SourceCheckFailure`].
pub fn check_file_diagnostic(
    source: &SourceFile,
    limits: crate::Limits,
) -> DiagnosticResult<CheckedSourceFile> {
    Ok(modules::check("", std::slice::from_ref(source), &[], limits)?.checked)
}

/// Carries the original enclosing item without a second source traversal.
pub(super) struct FunctionDeclaration<'a> {
    pub binding: &'a Binding,
    pub span: SpanId,
    /// The schema whose `has` block declares this method; `None` for free functions.
    pub receiver: Option<usize>,
    /// The operator written by `#infix` on this method.
    pub infix: Option<InfixOperator>,
    /// Copied into the type from a trait it provides.
    pub provided: bool,
    /// The type argument of a provided trait that this method's result must be.
    pub trait_result: Option<&'a Expression>,
    /// The trait whose bodiless method this is: the signature a call through a bound reads.
    pub abstract_trait: Option<&'a TypeDeclaration>,
}

/// Every parameterised `has` member is a method of the type, whether or not it takes
/// `self`; members without parameters are fields.
pub(super) fn is_method(binding: &Binding) -> bool {
    binding.parameters.is_some()
}

/// `self` names the receiver only as the first parameter.
pub(super) fn self_is_misplaced(binding: &Binding) -> bool {
    binding.parameters.as_ref().is_some_and(|parameters| {
        parameters
            .value
            .iter()
            .skip(1)
            .any(|parameter| parameter.value.name.value == "self")
    })
}

impl Checker<'_> {
    /// Name the culprit of `error` before returning it: `return Err(self.explain(..))`.
    pub(super) fn explain(&mut self, error: SourceCheckError, detail: String) -> SourceCheckError {
        self.detail = Some((error, detail));
        error
    }

    /// Record a non-fatal finding at `context`.
    pub(super) fn note(&mut self, kind: SourceCheckNoteKind, context: SpanId, detail: String) {
        self.note_with(kind, context, detail, Vec::new());
    }

    pub(super) fn note_with(
        &mut self,
        kind: SourceCheckNoteKind,
        context: SpanId,
        detail: String,
        facts: Vec<SourceFact>,
    ) {
        let span = self.span_of(context);
        let located = !span.is_empty() && self.text_of(context).is_some();
        // A body is checked once per world, and a finding about it is told once.
        if located
            && self
                .module
                .notes
                .iter()
                .any(|known| known.kind == kind && known.span == Some(span))
        {
            return;
        }
        self.module.notes.push(SourceCheckNote {
            kind,
            span: located.then_some(span),
            file: located.then(|| context.file() as usize),
            detail,
            facts,
        });
    }

    pub(super) fn item_count(&self) -> usize {
        self.files.iter().map(|file| file.items.len()).sum()
    }
}

impl Checker<'_> {
    /// Called only while propagating a failed phase operation, never while visiting
    /// tokens or updating proof state. Caught inner errors cannot leave stale context.
    pub(super) fn failure(&self, error: SourceCheckError, context: SpanId) -> SourceCheckFailure {
        let mut failure = SourceCheckFailure::from(error);
        // Only a detail written for this very error belongs to it; one left by an error
        // that was caught and recovered from must not describe a later failure.
        failure.detail = self
            .detail
            .as_ref()
            .filter(|(kind, _)| *kind == error)
            .map(|(_, text)| text.clone());
        if error != SourceCheckError::Budget {
            let span = self.span_of(context);
            if !span.is_empty() && self.text_of(context).is_some() {
                failure.span = Some(span);
                failure.file = Some(context.file() as usize);
                // The last expression visited, if it is inside the construct reported.
                failure.culprit = self
                    .culprit
                    .map(|id| self.span_of(id))
                    .filter(|inner| !inner.is_empty() && span.start() <= inner.start() && inner.end() <= span.end());
                failure.facts = self.fact_snapshot();
            }
        }
        failure
    }

    /// What the checker has proved about the named integers in scope right now: the interval of
    /// each, a known value, a multiple, an offset from another name, and relations between
    /// names. Names are those a person wrote; temporaries are left out.
    pub(super) fn fact_snapshot(&self) -> Vec<SourceFact> {
        let name_of = |identity: u32| {
            self.proof
                .names
                .iter()
                .find(|(_, value)| value.identity == Some(identity))
                .map(|(name, _)| (*name).to_owned())
        };
        let mut facts = Vec::new();
        for (name, value) in &self.proof.names {
            let SourceType::Integer { .. } = value.ty else { continue };
            let (mut lower, mut upper) = (value.lower, value.upper);
            if let Some(identity) = value.identity {
                if let Some((_, l, u)) = self.proof.scalars.iter().find(|(id, _, _)| *id == identity) {
                    lower = lower.max(*l);
                    upper = upper.min(*u);
                }
                for (id, modulus) in &self.proof.multiples {
                    if *id == identity {
                        facts.push(SourceFact::Multiple { name: (*name).to_owned(), modulus: *modulus });
                    }
                }
            }
            if lower == upper {
                facts.push(SourceFact::Known { name: (*name).to_owned(), value: lower });
            } else {
                facts.push(SourceFact::Interval { name: (*name).to_owned(), lower, upper });
            }
            if let Some((base, offset)) = value.affine {
                if let Some(base) = name_of(base).filter(|b| b != name) {
                    facts.push(SourceFact::Offset { name: (*name).to_owned(), base, offset });
                }
            }
        }
        for (left, operator, right) in &self.proof.facts {
            let symbol = match operator {
                BinaryOperator::Less => "<",
                BinaryOperator::LessOrEqual => "<=",
                BinaryOperator::Greater => ">",
                BinaryOperator::GreaterOrEqual => ">=",
                BinaryOperator::Equal => "=",
                _ => continue,
            };
            if let (Some(left), Some(right)) = (name_of(*left), name_of(*right)) {
                facts.push(SourceFact::Relation { left, operator: symbol, right });
            }
        }
        facts
    }
}

#[cfg(test)]
mod tests;
