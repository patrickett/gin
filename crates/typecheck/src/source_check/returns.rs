//! Returns have one optional expression; bare lists are ordinary sequence expressions.
use super::*;
use aggregates::EvalTask;

impl<'a> Checker<'a> {
    pub(super) fn return_tasks(
        &mut self,
        returned: &'a ReturnStatement,
        parent: SpanId,
        expected: Option<SourceType>,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        self.return_syntax(returned, parent)?;
        if let Some(value) = &returned.value {
            pending.push(EvalTask::Expression(value, expected, false));
        } else {
            pending.push(EvalTask::Unit);
        }
        Ok(())
    }

    fn return_syntax(&mut self, returned: &'a ReturnStatement, parent: SpanId) -> Result<()> {
        let text = self.span(parent)?;
        if !text.starts_with("return") {
            return Err(SourceCheckError::Malformed);
        }
        let range = self.range_of(parent);
        let mut cursor = range.start + "return".len();
        self.keyword_boundaries(FileRange {
            end: cursor,
            ..range
        })?;
        if let Some(value) = &returned.value {
            self.child(parent, value.span_id)?;
            let value = self.range_of(value.span_id);
            self.syntax_trivia(value.file, cursor, value.start)?;
            cursor = value.end;
        }
        self.syntax_trivia(range.file, cursor, range.end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn returned(source: &mut SourceFile) -> &mut ReturnStatement {
        let Item::Binding(binding) = &mut source.items[0].value else {
            unreachable!()
        };
        let Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value else {
            unreachable!()
        };
        let Statement::Return(returned) = &mut block.statements[0].value else {
            unreachable!()
        };
        returned
    }

    #[test]
    fn absent_return_values_cannot_erase_written_values_or_unit() {
        for text in [
            "f(): return 1, 2\n",
            "f(): return (1, 2)\n",
            "f(): return ()\n",
            "f() in 0 to 1: return 1\n",
        ] {
            let mut source = parser::parse_strict(text).unwrap();
            returned(&mut source).value = None;
            assert_eq!(
                check_file(&source, crate::Limits::default()),
                Err(SourceCheckError::Malformed),
                "{text}"
            );
        }
        let source = parser::parse_strict("f(): return\n").unwrap();
        assert!(check_file(&source, crate::Limits::default()).is_ok());
    }

    #[test]
    fn bare_sequence_nodes_require_original_order_and_at_least_two_elements() {
        for mutation in 0..4 {
            let mut source = parser::parse_strict("f(): return (), ()\n").unwrap();
            let Expr::Sequence(sequence) = &mut returned(&mut source).value.as_mut().unwrap().value
            else {
                unreachable!()
            };
            assert_eq!(sequence.delimiters, SequenceDelimiters::Bare);
            match mutation {
                0 => {
                    sequence.elements.clear();
                    sequence.separators.clear();
                }
                1 => {
                    sequence.elements.pop();
                    sequence.separators.clear();
                }
                2 => sequence.elements.swap(0, 1),
                3 => sequence.separators.clear(),
                _ => unreachable!(),
            }
            assert_eq!(
                check_file(&source, crate::Limits::default()),
                Err(SourceCheckError::Malformed),
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn bare_and_parenthesized_sequences_infer_the_same_positional_views() {
        let mut results = Vec::new();
        for text in ["f(): return 1, 2\n", "f(): return (1, 2)\n"] {
            let mut source = parser::parse_strict(text).unwrap();
            let value = returned(&mut source).value.as_ref().unwrap().clone();
            let mut checker = Checker::new(&source, crate::Limits::default());
            let result = checker.evaluate(&value, None).unwrap();
            let owner = result.owner.unwrap();
            assert_eq!(checker.proof.owners[owner].slots, [0, 1]);
            let SourceType::Sequence { id } = result.ty else {
                unreachable!()
            };
            results.push(checker.module.instances[id as usize].arguments.clone());
        }
        assert_eq!(results[0], results[1]);
    }

    #[test]
    fn an_empty_block_cannot_replace_a_written_sequence_value() {
        let mut source = parser::parse_strict("f(): return (1, 2)\n").unwrap();
        returned(&mut source).value.as_mut().unwrap().value =
            Expr::Block(Block { statements: vec![] });
        assert_eq!(
            check_file(&source, crate::Limits::default()),
            Err(SourceCheckError::Malformed)
        );
    }
}
