//! Heterogeneous fixed positions share arrays' sparse original child ledger.
use super::*;
use aggregates::EvalTask;
use contracts::{Layout, Origin};

impl<'a> Checker<'a> {
    pub(super) fn sequence_type(&mut self, elements: Vec<SourceType>) -> Result<SourceType> {
        self.intern_instance(CheckedSourceInstanceHead::Sequence, elements)
    }

    /// The record a function with several named results returns, if `ty` is one.
    pub(super) fn is_anonymous_record(&self, ty: SourceType) -> bool {
        let SourceType::Aggregate { id } = ty else {
            return false;
        };
        let schema = self.module.instances[id as usize].schema();
        self.module
            .anonymous_records
            .iter()
            .any(|(_, layout)| *layout == schema)
    }

    /// The layout of the record `binding` returns, built from its named results on first use.
    pub(super) fn anonymous_record(
        &mut self,
        binding: &'a Binding,
        fields: Vec<(String, SourceType)>,
    ) -> Result<SourceType> {
        let key = binding as *const Binding as usize;
        let owner = match self.proof.receiver_schema {
            Some(schema) => format!("{}.{}", self.module.schemas[schema].name.value, binding.name.value),
            None => binding.name.value.clone(),
        };
        let stable = format!("{owner}#record");
        self.anonymous_record_keyed(key, format!("{}.result", binding.name.value), stable, fields)
    }

    /// The stable key of the next labeled-array record made in the function being checked.
    pub(super) fn labeled_record_key(&self) -> String {
        let prefix = format!("{}#labeled#", self.module.current_function);
        let count = self
            .module
            .layouts
            .iter()
            .filter(|layout| layout.key.as_deref().is_some_and(|key| key.starts_with(&prefix)))
            .count();
        format!("{prefix}{count}")
    }

    /// The record type of exactly these fields, one per `key` (a binding, or a labeled array).
    pub(super) fn anonymous_record_keyed(
        &mut self,
        key: usize,
        name: String,
        stable: String,
        fields: Vec<(String, SourceType)>,
    ) -> Result<SourceType> {
        let existing = self
            .module
            .anonymous_records
            .iter()
            .find(|(other, _)| *other == key)
            .map(|(_, layout)| *layout);
        let layout = match existing {
            Some(layout) => layout,
            None => {
                self.charge(fields.len() + self.module.layouts.len())?;
                let layout = self.module.layouts.len();
                let owned = self.module.anonymous_records.len();
                let origin = Origin {
                    module: self.module.origin.clone(),
                    declaration: self.module.schemas.len() + self.module.union_variants.len() + owned,
                };
                let linear = fields.iter().any(|(_, ty)| {
                    !matches!(ty, SourceType::Integer { .. } | SourceType::Unit | SourceType::Text)
                });
                self.module.layouts.push(Layout {
                    origin,
                    schema: CheckedSourceAggregate {
                        name,
                        parameters: self.proof.generics.iter().map(|name| (*name).to_owned()).collect(),
                        fields: fields
                            .into_iter()
                            .map(|(name, ty)| CheckedSourceField {
                                name,
                                ty,
                                private: false,
                                register: None,
                                borrow: None,
                                length: None,
                            })
                            .collect(),
                        variants: Vec::new(),
                        capability: if linear {
                            Capability::Linear
                        } else {
                            Capability::Unrestricted
                        },
                        kind: AggregateKind::Record,
                        wrapping: false,
                        defaults: Vec::new(),
                        bounds: Vec::new(),
                        provides: Vec::new(),
                        borrowing: Borrowing::None,
                        numbers: Vec::new(),
                        domain_claims: Vec::new(),
                        discriminant: None,
                    },
                    constraints: Vec::new(),
                    key: Some(stable),
                });
                self.module.anonymous_records.push((key, layout));
                layout
            }
        };
        let arguments = (0..self.proof.generics.len())
            .map(|index| SourceType::Generic { index: index as u32 })
            .collect();
        self.instance(layout, arguments)
    }

    /// The type of position `slot` of a sequence, an array, or an anonymous record.
    fn slot_type(&mut self, ty: SourceType, slot: u32) -> Result<SourceType> {
        if let SourceType::Aggregate { id } = ty
            && self.is_anonymous_record(ty)
        {
            let fields = self.instance_fields(id)?;
            return fields.get(slot as usize).copied().ok_or(SourceCheckError::Type);
        }
        self.position_type(ty, slot)
    }

    pub(super) fn position_count(&self, ty: SourceType) -> Result<u32> {
        match ty {
            SourceType::Array { .. } => Ok(self.array_layout(ty)?.1),
            SourceType::Sequence { id } => {
                let instance = &self.module.instances[id as usize];
                assert_eq!(instance.head, CheckedSourceInstanceHead::Sequence);
                assert!(!instance.arguments.is_empty());
                u32::try_from(instance.arguments.len()).map_err(|_| SourceCheckError::Budget)
            }
            _ => Err(SourceCheckError::Type),
        }
    }

    pub(super) fn position_type(&self, ty: SourceType, slot: u32) -> Result<SourceType> {
        if slot >= self.position_count(ty)? {
            return Err(SourceCheckError::Type);
        }
        match ty {
            SourceType::Array { .. } => Ok(self.array_layout(ty)?.0),
            SourceType::Sequence { id } => {
                Ok(self.module.instances[id as usize].arguments[slot as usize])
            }
            _ => unreachable!("only structural positions have a count"),
        }
    }

    pub(super) fn group_syntax(
        &mut self,
        expression: &'a Expression,
        child: &'a Expression,
    ) -> Result<()> {
        self.child(expression.span_id, child.span_id)?;
        let text = self.span(expression.span_id)?;
        if !text.starts_with('(') || !text.ends_with(')') {
            return Err(SourceCheckError::Malformed);
        }
        let parent = self.range_of(expression.span_id);
        let child = self.range_of(child.span_id);
        self.syntax_trivia(child.file, parent.start + 1, child.start)?;
        self.syntax_trivia(child.file, child.end, parent.end - 1)?;
        Ok(())
    }

    /// Gaps may contain only the lexer's whitespace or completed `--` line comments.
    /// A comment cannot end at a following syntax node: that node would be commented out.
    pub(super) fn syntax_trivia(&mut self, file: u32, start: usize, end: usize) -> Result<()> {
        self.syntax_trivia_inner(file, start, end, false)
    }

    pub(super) fn syntax_trivia_to_eof(&mut self, file: u32, start: usize) -> Result<()> {
        let end = self
            .files
            .get(file as usize)
            .map_or(0, |file| file.text.len());
        self.syntax_trivia_inner(file, start, end, true)
    }

    /// The written text between two offsets of file `file`.
    pub(super) fn gap_text(&self, file: u32, start: usize, end: usize) -> Result<&'a str> {
        self.files
            .get(file as usize)
            .and_then(|file| file.text.get(start..end))
            .ok_or(SourceCheckError::Malformed)
    }

    fn syntax_trivia_inner(
        &mut self,
        file: u32,
        start: usize,
        end: usize,
        eof: bool,
    ) -> Result<()> {
        let text = self.gap_text(file, start, end)?;
        self.charge(text.len())?;
        let bytes = text.as_bytes();
        let mut index = 0;
        let mut comment = false;
        while index < bytes.len() {
            if comment {
                if bytes[index] == b'\n' {
                    comment = false;
                }
                index += 1;
            } else {
                match bytes[index] {
                    b' ' | b'\t' | b'\n' => index += 1,
                    b'-' if bytes.get(index + 1) == Some(&b'-') => {
                        comment = true;
                        index += 2;
                    }
                    _ => return Err(SourceCheckError::Malformed),
                }
            }
        }
        if comment && !eof {
            return Err(SourceCheckError::Malformed);
        }
        Ok(())
    }

    pub(super) fn sequence_syntax(
        &mut self,
        expression: &'a Expression,
        sequence: &'a SequenceExpression,
    ) -> Result<()> {
        let (mut cursor, end) = self.sequence_bounds(expression, sequence)?;
        let expression_file = expression.span_id.file();
        let count = sequence.elements.len();
        let separators = sequence.separators.len();
        if (count == 0 && separators != 0)
            || (count == 1 && separators != 1)
            || (count > 1 && separators != count - 1 && separators != count)
        {
            return Err(SourceCheckError::Malformed);
        }
        self.charge(count + separators)?;
        for (index, element) in sequence.elements.iter().enumerate() {
            self.child(expression.span_id, element.span_id)?;
            let element = self.range_of(element.span_id);
            self.syntax_trivia(element.file, cursor, element.start)?;
            cursor = element.end;
            if let Some(separator) = sequence.separators.get(index) {
                self.child(expression.span_id, *separator)?;
                self.token(*separator, ",")?;
                let separator = self.range_of(*separator);
                self.syntax_trivia(separator.file, cursor, separator.start)?;
                cursor = separator.end;
            }
        }
        self.syntax_trivia(expression_file, cursor, end)?;
        Ok(())
    }

    fn sequence_bounds(
        &mut self,
        expression: &'a Expression,
        sequence: &'a SequenceExpression,
    ) -> Result<(usize, usize)> {
        let parent = self.range_of(expression.span_id);
        match sequence.delimiters {
            SequenceDelimiters::Parenthesized { open, close } => {
                self.child(expression.span_id, open)?;
                self.child(expression.span_id, close)?;
                self.token(open, "(")?;
                self.token(close, ")")?;
                let open = self.range_of(open);
                let close = self.range_of(close);
                if parent.start != open.start || parent.end != close.end {
                    return Err(SourceCheckError::Malformed);
                }
                Ok((open.end, close.start))
            }
            SequenceDelimiters::Bare => {
                let count = sequence.elements.len();
                if count < 2 || sequence.separators.len() != count - 1 {
                    return Err(SourceCheckError::Malformed);
                }
                Ok((parent.start, parent.end))
            }
        }
    }

    pub(super) fn numeric_select_index(
        &mut self,
        expression: &'a Expression,
        select: &'a NumericSelectExpression,
    ) -> Result<I256> {
        self.child(expression.span_id, select.base.span_id)?;
        self.child(expression.span_id, select.dot)?;
        self.child(expression.span_id, select.index.span_id)?;
        self.token(select.dot, ".")?;
        if !matches!(select.index.value, Expr::Literal(LiteralKind::Integer)) {
            return Err(SourceCheckError::Malformed);
        }
        let text = self.span(select.index.span_id)?;
        self.integer(text, false)
    }

    pub(super) fn sequence_tasks(
        &mut self,
        elements: &'a [Expression],
        expected: Option<SourceType>,
        pending: &mut Vec<EvalTask<'a>>,
    ) -> Result<()> {
        self.charge(elements.len() + 1)?;
        match expected {
            Some(SourceType::Unit) if elements.is_empty() => {}
            Some(ty @ SourceType::Sequence { .. })
                if self.position_count(ty)? as usize == elements.len() => {}
            Some(SourceType::Aggregate { id })
                if self.is_anonymous_record(SourceType::Aggregate { id })
                    && self.instance_fields(id)?.len() == elements.len() => {}
            Some(_) => return Err(SourceCheckError::Type),
            None => {}
        }
        pending.push(EvalTask::Sequence(elements.len(), expected));
        for (slot, child) in elements.iter().enumerate().rev() {
            let ty = expected
                .map(|ty| self.slot_type(ty, slot as u32))
                .transpose()?;
            pending.push(EvalTask::Expression(child, ty, false));
        }
        Ok(())
    }

    pub(super) fn sequence_pack(
        &mut self,
        mut children: Vec<Value>,
        expected: Option<SourceType>,
    ) -> Result<Value> {
        // `return a, b` where the function returns the record of its named results.
        if let Some(ty @ SourceType::Aggregate { id }) = expected
            && self.is_anonymous_record(ty)
        {
            let typed = self.instance_fields(id)?;
            if typed.len() != children.len() {
                return Err(SourceCheckError::Type);
            }
            for (child, field) in children.iter_mut().zip(&typed) {
                *child = self.view_as(*child, *field)?;
                self.release(*child)?;
            }
            let value = self.fresh_value(ty)?;
            let owner = value.owner.ok_or(SourceCheckError::Type)?;
            self.proof.owners[owner].children = children;
            self.proof.owners[owner].materialized = true;
            self.obligate(owner);
            return Ok(value);
        }
        if children.is_empty() {
            assert!(expected.is_none() || expected == Some(SourceType::Unit));
            return Ok(Value::unit());
        }
        let count = u32::try_from(children.len()).map_err(|_| SourceCheckError::Budget)?;
        self.charge(children.len() * 2)?;
        let mut elements = Vec::new();
        for (slot, child) in children.iter_mut().enumerate() {
            let ty = match expected {
                Some(ty) => self.position_type(ty, slot as u32)?,
                None => arrays::array_element_view(*child),
            };
            *child = self.view_as(*child, ty)?;
            self.release(*child)?;
            elements.push(ty);
        }
        let ty = self.sequence_type(elements)?;
        let value = self.fresh_value(ty)?;
        let owner = value
            .owner
            .expect("positive sequences own their positional root");
        self.proof.owners[owner].slots = (0..count).collect();
        self.proof.owners[owner].children = children;
        self.proof.owners[owner].materialized = true;
        self.obligate(owner);
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn returned_expression(source: &mut SourceFile) -> &mut Expression {
        let Item::Binding(binding) = &mut source.items.last_mut().unwrap().value else {
            unreachable!()
        };
        let Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value else {
            unreachable!()
        };
        let Statement::Return(ret) = &mut block.statements[0].value else {
            unreachable!()
        };
        ret.value.as_mut().unwrap()
    }

    #[test]
    fn manual_sequence_nodes_cannot_erase_or_reorder_written_elements_and_commas() {
        for text in ["f(): return (1, 2)\n", "f(): return (1, 2,)\n"] {
            for mutation in 0..4 {
                let mut source = parser::parse_strict(text).unwrap();
                let Expr::Sequence(sequence) = &mut returned_expression(&mut source).value else {
                    unreachable!()
                };
                match mutation {
                    0 => {
                        sequence.elements.clear();
                        sequence.separators.clear();
                    }
                    1 => sequence.elements.swap(0, 1),
                    2 => {
                        sequence.separators.clear();
                    }
                    3 => {
                        sequence.elements.pop();
                        sequence.separators.truncate(1);
                    }
                    _ => unreachable!(),
                }
                assert_eq!(
                    check_file(&source, crate::Limits::default()),
                    Err(SourceCheckError::Malformed),
                    "{text}, mutation {mutation}"
                );
            }
        }
    }

    #[test]
    fn manual_group_cannot_reclassify_a_written_singleton_sequence() {
        let mut source =
            parser::parse_strict("Marker has\nf() Marker: return (Marker,)\n").unwrap();
        let expression = returned_expression(&mut source);
        let Expr::Sequence(sequence) = &mut expression.value else {
            unreachable!()
        };
        let child = sequence.elements.pop().unwrap();
        expression.value = Expr::Group(Box::new(child));
        assert_eq!(
            check_file(&source, crate::Limits::default()),
            Err(SourceCheckError::Malformed)
        );
    }

    #[test]
    fn a_comma_inside_a_comment_cannot_turn_a_group_into_a_sequence() {
        let mut source = parser::parse_strict(concat!(
            "Marker has\n",
            "f() (Marker,): return (Marker -- ,\n",
            ")\n",
        ))
        .unwrap();
        let expression = returned_expression(&mut source);
        let Expr::Group(child) = &expression.value else {
            unreachable!()
        };
        let child = (**child).clone();
        let span = expression.span_id;
        let range = source.span_table.get(span).to_range();
        let comma = source.text.rfind(',').unwrap();
        let sequence = SequenceExpression {
            delimiters: SequenceDelimiters::Parenthesized {
                open: source.span_table.insert_range(range.start..range.start + 1),
                close: source.span_table.insert_range(range.end - 1..range.end),
            },
            elements: vec![child],
            separators: vec![source.span_table.insert_range(comma..comma + 1)],
        };
        returned_expression(&mut source).value = Expr::Sequence(Box::new(sequence));
        assert_eq!(
            check_file(&source, crate::Limits::default()),
            Err(SourceCheckError::Malformed)
        );
    }

    #[test]
    fn parsed_sequence_and_group_comments_keep_their_distinct_syntax() {
        let text = concat!(
            "f(element Type, value element) (element,):\n",
            "  return ( -- before the element\n",
            "    ( -- before the group child\n",
            "      value -- after the group child\n",
            "    ), -- after the comma\n",
            "  )\n",
            "unit() (): return ( -- no element\n",
            ")\n",
        );
        let source = parser::parse_strict(text).unwrap();
        assert!(check_file(&source, crate::Limits::default()).is_ok());
    }

    #[test]
    fn group_gaps_allow_only_bounded_lexer_trivia_and_completed_comments() {
        for (text, valid) in [
            ("(0)", true),
            ("( \t0\n)", true),
            ("( -- before , )\n0 -- after (,)\n)", true),
            ("(0,)", false),
            ("(0 1)", false),
            ("(--0)", false),
            ("(0 -- after)", false),
            ("(0\r)", false),
        ] {
            let mut source = parser::parse_strict("").unwrap();
            source.text = text.into();
            let start = text.find('0').unwrap();
            let child = Spanned::new(
                Expr::Literal(LiteralKind::Integer),
                source.span_table.insert_range(start..start + 1),
            );
            let expression = Spanned::new(
                Expr::Group(Box::new(child.clone())),
                source.span_table.insert_range(0..text.len()),
            );
            let mut checker = Checker::new(&source, crate::Limits::default());
            assert_eq!(
                checker.group_syntax(&expression, &child),
                if valid {
                    Ok(())
                } else {
                    Err(SourceCheckError::Malformed)
                },
                "{text}"
            );
        }
        let text = format!("probe: ({}0)\n", " ".repeat(1024));
        let source = parser::parse_strict(&text).unwrap();
        assert_eq!(
            check_file(
                &source,
                crate::Limits {
                    work_max: 1000,
                    ..crate::Limits::default()
                }
            ),
            Err(SourceCheckError::Budget)
        );
    }

    #[test]
    fn sequence_heads_are_heterogeneous_and_share_the_type_arena() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        let generic = SourceType::Generic { index: 0 };
        let integer = Value::integer(I256::ZERO, I256::ONE).ty;
        assert_eq!(checker.sequence_type(vec![]).unwrap(), SourceType::Unit);
        assert!(checker.module.instances.is_empty());
        let sequence = checker.sequence_type(vec![generic, integer]).unwrap();
        let repeated = checker.sequence_type(vec![generic, integer]).unwrap();
        assert_eq!(sequence, repeated);
        let array = checker.array_type(generic, 2).unwrap();
        assert_ne!(sequence, array);
        assert_eq!(checker.position_type(sequence, 0).unwrap(), generic);
        assert_eq!(checker.position_type(sequence, 1).unwrap(), integer);
        assert_eq!(
            checker.position_type(sequence, 2),
            Err(SourceCheckError::Type)
        );
        let narrower = checker
            .sequence_type(vec![generic, Value::integer(I256::ONE, I256::ONE).ty])
            .unwrap();
        assert!(checker.compatible(sequence, narrower).is_ok());
        assert_eq!(
            checker.compatible(narrower, sequence),
            Err(SourceCheckError::Type)
        );
        assert_eq!(
            checker.compatible(array, sequence),
            Err(SourceCheckError::Type)
        );
    }

    #[test]
    fn sparse_sequence_origins_are_not_hypothetical_output_owners() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        let generic = SourceType::Generic { index: 0 };
        let integer = Value::integer(I256::ZERO, I256::ONE).ty;
        let ty = checker.sequence_type(vec![generic, integer]).unwrap();
        let value = checker.input_value(ty).unwrap();
        checker.proof.names.push(("values", value));
        let inputs = checker.summary_inputs().unwrap();
        let summary = checker.summarize(&inputs, Some(value)).unwrap();
        assert_eq!(checker.proof.owners.len(), 1);
        assert_eq!(summary.inputs.len(), 1);
        assert_eq!(summary.outputs.len(), 1);
        assert_eq!(summary.outputs[0].input, Some(0));
        assert!(summary.inputs[0].children.is_empty());
        let scalar = checker.array_child(value, 1).unwrap();
        let repeated = checker.array_child(value, 1).unwrap();
        assert_eq!(scalar.identity, repeated.identity);
        assert_eq!(checker.proof.owners.len(), 1);
        let child = checker.array_child(value, 0).unwrap();
        assert_eq!(checker.proof.owners.len(), 2);
        let repeated = checker.array_child(value, 0).unwrap();
        assert_eq!(child.owner, repeated.owner);
        let inputs = checker.summary_inputs().unwrap();
        assert_eq!(inputs.nodes[0].slots, [0, 1]);
        assert_eq!(inputs.nodes.len(), 3);
    }

    #[test]
    fn sequence_inference_keeps_scalar_domains_and_original_owners() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        let scalar = checker
            .fresh_value(Value::integer(I256::ZERO, I256::ONE).ty)
            .unwrap();
        checker.guard_integer(scalar, I256::ZERO).unwrap();
        let narrowed = checker.scalar_facts(scalar).unwrap();
        let owned = checker
            .input_value(SourceType::Generic { index: 0 })
            .unwrap();
        let sequence = checker.sequence_pack(vec![narrowed, owned], None).unwrap();
        let scalar_child = checker.array_child(sequence, 0).unwrap();
        assert_eq!(scalar_child.identity, scalar.identity);
        assert_eq!(scalar_child.integer_domain, scalar.integer_domain);
        assert_eq!(
            (scalar_child.lower, scalar_child.upper),
            (I256::ZERO, I256::ZERO)
        );
        let owned_child = checker.array_child(sequence, 1).unwrap();
        assert_eq!(owned_child.owner, owned.owner);
        assert_eq!(owned_child.identity, owned.identity);
        assert!(!checker.proof.owners[sequence.owner.unwrap()].drained);
    }

    #[test]
    fn mixed_projection_spellings_copy_scalars_and_transfer_owned_positions_once() {
        let source =
            parser::parse_strict("a: values.0\nb: values.[0]\nc: values.1\nd: values.[1]\n")
                .unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        let ty = checker
            .sequence_type(vec![
                Value::integer(I256::ZERO, I256::ONE).ty,
                SourceType::Generic { index: 0 },
            ])
            .unwrap();
        let root = checker.input_value(ty).unwrap();
        checker.insert("values", root).unwrap();
        let mut returned = Vec::new();
        for item in &source.items {
            let Item::Binding(binding) = &item.value else {
                unreachable!()
            };
            returned.push(checker.evaluate(&binding.value.as_ref().unwrap().expression, None));
        }
        let first = returned[0].as_ref().unwrap();
        let repeated = returned[1].as_ref().unwrap();
        assert_eq!(first.identity, repeated.identity);
        assert_eq!(first.integer_domain, repeated.integer_domain);
        assert!(returned[2].is_ok());
        assert!(matches!(returned[3], Err(SourceCheckError::UseAfterMove)));
        assert_eq!(
            checker.array_complete(root),
            Err(SourceCheckError::UseAfterMove)
        );
        let owner = root.owner.unwrap();
        assert_eq!(checker.proof.owners[owner].moved, [1]);
        assert!(!checker.proof.owners[owner].drained);
        assert!(checker.proof.owners[owner].live);
    }

    #[test]
    fn recursive_sequence_views_survive_inference_opaque_calls_and_import_compaction() {
        let library_text = concat!(
            "Big is A or B or C\nFlag is Seen\n",
            "Box(element) has value element\n",
            "opaque(element Type, value element) element: return value\n",
            "make() (Big, [Big; 1], (Big,)): return (A, [A], (A,))\n",
        );
        let library_source = parser::parse_strict(library_text).unwrap();
        let library =
            check_module("library", &library_source, &[], crate::Limits::default()).unwrap();
        drop(library_source);
        let prefix = concat!(
            "use library Big, Flag, Box, opaque, make\n",
            "observe() (Flag, [Big; 1], (Big,)):\n",
            "  wide: make()\n",
            "  carried: opaque((Big, [Big; 1], (Big,)), wide)\n",
            "  boxed: Box(value: carried)\n",
            "  Box(values) := boxed\n",
            "  value: values.0\n",
            "  array: values.[1]\n",
            "  nested: values.2\n",
            "  observed Flag: when value is\n",
            "    A then Seen\n",
            "    B then Seen\n",
        );
        let incomplete = format!("{prefix}  return (observed, array, nested)\n");
        let source = parser::parse_strict(&incomplete).unwrap();
        assert_eq!(
            check_module("client", &source, &[&library], crate::Limits::default()).unwrap_err(),
            SourceCheckError::Type
        );
        let complete = format!("{prefix}    C then Seen\n  return (observed, array, nested)\n");
        let source = parser::parse_strict(&complete).unwrap();
        let client =
            check_module("client", &source, &[&library], crate::Limits::default()).unwrap();
        assert_eq!(client.summaries("observe").len(), 1);
    }

    #[test]
    fn nested_array_sequence_views_preserve_declared_sum_coverage() {
        for (big, literal, extraction) in [
            (
                "([Big; 1],)",
                "([A],)",
                "  row: values.0\n  value: row.[0]\n",
            ),
            (
                "[(Big,); 1]",
                "[(A,)]",
                "  row: values.[0]\n  value: row.0\n",
            ),
            (
                "((Big,),)",
                "((A,),)",
                "  row: values.0\n  value: row.0\n",
            ),
        ] {
            let library_text = format!(
                "Big is A or B or C\nFlag is Seen\n\
                 Box(element) has value element\n\
                 make() {big}: return {literal}\n"
            );
            let source = parser::parse_strict(&library_text).unwrap();
            let library = check_module("library", &source, &[], crate::Limits::default()).unwrap();
            let bindings = concat!(
                "  wide: make()\n",
                "  boxed: Box(value: wide)\n",
                "  Box(values) := boxed\n",
            );
            let prefix = format!(
                "use library Big, Flag, Box, make\nobserve() Flag:\n\
                 {bindings}{extraction}  seen Flag: when value is\n    A then Seen\n    B then Seen\n"
            );
            let source = parser::parse_strict(&format!("{prefix}  return seen\n")).unwrap();
            assert_eq!(
                check_module("client", &source, &[&library], crate::Limits::default()).unwrap_err(),
                SourceCheckError::Type
            );
            let complete = format!("{prefix}    C then Seen\n  return seen\n");
            let source = parser::parse_strict(&complete).unwrap();
            let client =
                check_module("client", &source, &[&library], crate::Limits::default()).unwrap();
            assert_eq!(client.summaries("observe").len(), 1);
        }
    }
}
