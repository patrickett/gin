use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use ast::{BindValue, ConstValue, FileAst};

use crate::DefId;
use crate::analysis::{CompTimeEvaluator, ConstEnv};

type Answer = Option<ConstValue>;
type AnswerCell = Arc<OnceLock<Answer>>;

pub(crate) struct ConstantQueries {
    ast: Arc<FileAst>,
    target_layout: Option<crate::layout::TargetLayout>,
    environment: OnceLock<ConstEnv>,
    answers: Mutex<HashMap<DefId, AnswerCell>>,
    evaluations: AtomicUsize,
    environments: AtomicUsize,
}

impl ConstantQueries {
    pub fn new(ast: Arc<FileAst>) -> Self {
        Self::for_layout(ast, None)
    }

    pub fn for_layout(
        ast: Arc<FileAst>,
        target_layout: Option<crate::layout::TargetLayout>,
    ) -> Self {
        Self {
            ast,
            target_layout,
            environment: OnceLock::new(),
            answers: Mutex::new(HashMap::new()),
            evaluations: AtomicUsize::new(0),
            environments: AtomicUsize::new(0),
        }
    }

    pub fn stats(&self) -> (usize, usize) {
        (
            self.evaluations.load(Ordering::Relaxed),
            self.environments.load(Ordering::Relaxed),
        )
    }

    pub fn value(&self, id: DefId) -> Answer {
        let cell = self
            .answers
            .lock()
            .expect("constant query cache poisoned")
            .entry(id)
            .or_default()
            .clone();
        let answer = cell
            .get_or_init(|| {
                self.evaluations.fetch_add(1, Ordering::Relaxed);
                let bind = self.ast.defs.get(&id)?;
                let BindValue::Expr(expr) = &bind.value else {
                    return None;
                };
                expr.const_value.as_deref().cloned().or_else(|| {
                    let environment = self.environment.get_or_init(|| {
                        self.environments.fetch_add(1, Ordering::Relaxed);
                        crate::prepare_target::const_env_from_prepared_ast(&self.ast, None)
                    });
                    CompTimeEvaluator::for_layout(environment, &self.ast, self.target_layout)
                        .eval(&expr.value)
                })
            })
            .clone();
        super::finish_reads::record_constant(id, &answer);
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ast::prelude::Intern;
    use parser::query::SourceParseExt;

    #[test]
    fn query_answers_preserve_the_existing_environment_and_evaluator() {
        for source in [
            "seed: 1\n",
            "seed: compute(0)\ncompute(x): return 2\n",
            "seed: later\nlater: 3\n",
            "seed: missing(0)\n",
            "seed: next\nnext: seed\n",
            "seed: recur(0)\nrecur(x): return recur(x)\n",
        ] {
            let parsed = source.parse_source_full();
            assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
            let ast = Arc::new(parsed.ast);
            let queries = ConstantQueries::new(Arc::clone(&ast));
            let environment = crate::prepare_target::const_env_from_prepared_ast(&ast, None);
            let evaluator = CompTimeEvaluator::new(&environment, &ast);
            for (id, bind) in &ast.defs {
                let expected = match &bind.value {
                    BindValue::Expr(expr) => expr
                        .const_value
                        .as_deref()
                        .cloned()
                        .or_else(|| evaluator.eval(&expr.value)),
                    _ => None,
                };
                assert_eq!(queries.value(*id), expected);
                assert_eq!(queries.value(*id), expected);
            }
            assert_eq!(
                queries.value(DefId::named(Intern::from_ref("absent"))),
                None
            );
            assert_eq!(queries.stats().0, ast.defs.len() + 1);
            assert!(queries.stats().1 <= 1);
        }
    }

    #[test]
    fn prepared_answers_and_missing_definitions_do_not_build_an_environment() {
        let mut parsed = "seed: 1\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let BindValue::Expr(expr) = &mut parsed
            .ast
            .defs
            .get_mut(&DefId::named(Intern::from_ref("seed")))
            .expect("seed definition")
            .value
        else {
            panic!("seed expression")
        };
        expr.const_value = Some(Box::new(ConstValue::Int(1.into())));
        let queries = ConstantQueries::new(Arc::new(parsed.ast));
        for _ in 0..4 {
            assert_eq!(
                queries.value(DefId::named(Intern::from_ref("seed"))),
                Some(ConstValue::Int(1.into()))
            );
            assert_eq!(
                queries.value(DefId::named(Intern::from_ref("absent"))),
                None
            );
        }
        assert_eq!(queries.stats(), (2, 0));
    }

    #[test]
    fn concurrent_distinct_answers_share_one_environment() {
        let parsed =
            "first: compute(0)\nsecond: compute(0)\ncompute(x): return 2\n".parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let queries = ConstantQueries::new(Arc::new(parsed.ast));
        std::thread::scope(|scope| {
            for name in ["first", "second", "first", "second"] {
                let queries = &queries;
                scope.spawn(move || {
                    for _ in 0..16 {
                        assert_eq!(
                            queries.value(DefId::named(Intern::from_ref(name))),
                            Some(ConstValue::Int(2.into()))
                        );
                    }
                });
            }
        });
        assert_eq!(queries.stats(), (2, 1));
    }

    #[test]
    fn concurrent_queries_share_one_environment_and_answer() {
        let ast = Arc::new(
            "seed: compute(0)\ncompute(x): return 2\n"
                .parse_source_full()
                .ast,
        );
        let queries = ConstantQueries::new(ast);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..16 {
                        assert_eq!(
                            queries.value(DefId::named(Intern::from_ref("seed"))),
                            Some(ConstValue::Int(2.into()))
                        );
                    }
                });
            }
        });
        assert_eq!(queries.stats(), (1, 1));
    }
}
