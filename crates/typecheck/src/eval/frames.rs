use super::*;
use crate::{DefId, FileId};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EvalFrame {
    pub function: Option<DefId>,
    pub file: FileId,
    pub expression: Option<ExprId>,
    pub locals: Vec<EvalLocal>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EvalLocal {
    pub name: Intern<String>,
    pub value: EvalResult<ConstValue>,
    pub is_reference: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EvalFailure {
    pub diagnostic: Diagnostic,
    pub frames: Vec<EvalFrame>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EvalCondition {
    pub expression: ExprId,
    pub failure: EvalFailure,
}

impl From<Diagnostic> for EvalFailure {
    fn from(diagnostic: Diagnostic) -> Self {
        Self {
            diagnostic,
            frames: Vec::new(),
        }
    }
}

impl crate::TypedFileAst {
    pub(crate) fn record_eval_failure(&mut self, expression: ExprId, mut failure: EvalFailure) {
        if failure.diagnostic.span.is_empty() {
            failure.diagnostic.span = self.span_table.get(self.exprs.span[expression.index()]);
        }
        if self.eval_conditions.iter().any(|condition| {
            condition.expression == expression && condition.failure.diagnostic == failure.diagnostic
        }) {
            return;
        }
        self.exprs.facts.semantic_facts.flaws[expression.index()].push(failure.diagnostic.clone());
        self.eval_conditions.push(EvalCondition {
            expression,
            failure,
        });
    }
}

pub(super) struct ActiveFrame {
    pub function: Option<DefId>,
    pub file: FileId,
    pub expression: Option<ExprId>,
    pub caller_locals: Vec<HashMap<Intern<String>, LocalValue>>,
}

impl ActiveFrame {
    pub fn expression(file: FileId) -> Self {
        Self {
            function: None,
            file,
            expression: None,
            caller_locals: Vec::new(),
        }
    }
}

impl<H: EvalHost> Evaluator<'_, H> {
    pub fn take_failure(&mut self, diagnostic: Diagnostic) -> EvalFailure {
        EvalFailure {
            diagnostic,
            frames: self.take_failure_frames(),
        }
    }

    pub fn failure_frames(&self) -> &[EvalFrame] {
        self.failure_frames.as_deref().unwrap_or_default()
    }

    pub fn take_failure_frames(&mut self) -> Vec<EvalFrame> {
        self.failure_frames.take().unwrap_or_default()
    }

    pub(super) fn capture_frames(&mut self) {
        if self.failure_frames.is_some() {
            return;
        }
        let frames = self
            .frames
            .iter()
            .enumerate()
            .rev()
            .map(|(index, frame)| {
                let scopes = self
                    .frames
                    .get(index + 1)
                    .map(|callee| &callee.caller_locals)
                    .unwrap_or(&self.locals);
                let bindings = scopes
                    .iter()
                    .flat_map(|scope| scope.iter())
                    .collect::<HashMap<_, _>>();
                let mut locals = bindings
                    .into_iter()
                    .map(|(name, local)| {
                        let (address, is_reference) = match local {
                            LocalValue::Stored(address) => (*address, false),
                            LocalValue::Reference(address) => (*address, true),
                        };
                        EvalLocal {
                            name: *name,
                            value: self.heap.freeze(address),
                            is_reference,
                        }
                    })
                    .collect::<Vec<_>>();
                locals.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
                EvalFrame {
                    function: frame.function,
                    file: frame.file,
                    expression: frame.expression,
                    locals,
                }
            })
            .collect();
        self.failure_frames = Some(frames);
    }
}
