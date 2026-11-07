//! Structural fixed arrays: sparse original slot origins and owned-local transfers.
use super::*;
use aggregates::EvalTask;

impl<'a> Checker<'a> {
    pub(super) fn array_type(&mut self, element: SourceType, extent: u32) -> Result<SourceType> {
        self.intern_instance(CheckedSourceInstanceHead::Array { extent }, vec![element])
    }

    pub(super) fn array_layout(&self, ty: SourceType) -> Result<(SourceType, u32)> {
        let SourceType::Array { id } = ty else {
            return Err(SourceCheckError::Type);
        };
        let instance = &self.module.instances[id as usize];
        // A slice's extent belongs to each value; as a type it is as long as can be.
        let extent = match instance.head {
            CheckedSourceInstanceHead::Array { extent } => extent,
            CheckedSourceInstanceHead::Slice | CheckedSourceInstanceHead::Sized => u32::MAX,
            _ => unreachable!("array type has an array head"),
        };
        assert!(matches!(instance.arguments.len(), 1 | 2));
        Ok((instance.arguments[0], extent))
    }

    pub(super) fn array_type_extent(
        &mut self,
        expression: &'a Expression,
        array: &'a ArrayExpression,
    ) -> Result<u32> {
        self.array_syntax(expression, array)?;
        if array.elements.len() != 1 {
            return Err(SourceCheckError::Type);
        }
        let length = array.length.as_ref().ok_or(SourceCheckError::Unsupported)?;
        let number = self.array_constant(&length.value)?;
        if number < I256::ZERO {
            return Err(SourceCheckError::Type);
        }
        if number > I256::from(u32::MAX) {
            return Err(SourceCheckError::Budget);
        }
        Ok(number.as_u32())
    }

    pub(super) fn array_syntax(
        &mut self,
        expression: &'a Expression,
        array: &'a ArrayExpression,
    ) -> Result<()> {
        self.child(expression.span_id, array.open)?;
        self.child(expression.span_id, array.close)?;
        self.token(array.open, "[")?;
        self.token(array.close, "]")?;
        self.charge(array.elements.len() + array.separators.len())?;
        for element in &array.elements {
            self.child(expression.span_id, element.span_id)?;
        }
        for separator in &array.separators {
            self.child(expression.span_id, *separator)?;
            self.token(*separator, ",")?;
        }
        if let Some(length) = &array.length {
            self.child(expression.span_id, length.separator)?;
            self.child(expression.span_id, length.value.span_id)?;
            self.token(length.separator, ";")?;
        }
        Ok(())
    }

    /// Closed integer expressions only: no local facts or calls enter type caching.
    pub(super) fn array_constant(&mut self, root: &'a Expression) -> Result<I256> {
        self.charge(1)?;
        let mut tasks = vec![(root, false)];
        let mut values = Vec::new();
        while let Some((expression, visited)) = tasks.pop() {
            self.charge(1)?;
            match &expression.value {
                Expr::Group(child) => {
                    self.group_syntax(expression, child)?;
                    tasks.push((child, false));
                }
                Expr::Binary {
                    left,
                    operator,
                    right,
                } => {
                    self.child(expression.span_id, left.span_id)?;
                    self.child(expression.span_id, right.span_id)?;
                    self.child(expression.span_id, operator.span_id)?;
                    if visited {
                        let right = values.pop().ok_or(SourceCheckError::Malformed)?;
                        let left = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(self.array_constant_binary(operator, left, right)?);
                    } else {
                        self.charge(3)?;
                        tasks.push((expression, true));
                        tasks.push((right, false));
                        tasks.push((left, false));
                    }
                }
                Expr::Unary { operator, operand } => {
                    self.child(expression.span_id, operand.span_id)?;
                    self.child(expression.span_id, operator.span_id)?;
                    self.token(operator.span_id, "-")?;
                    if matches!(operand.value, Expr::Literal(LiteralKind::Integer)) {
                        let text = self.span(operand.span_id)?;
                        values.push(self.integer(text, true)?);
                    } else if visited {
                        let value: I256 = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(value.checked_neg().ok_or(SourceCheckError::Overflow)?);
                    } else {
                        self.charge(2)?;
                        tasks.push((expression, true));
                        tasks.push((operand, false));
                    }
                }
                Expr::Literal(LiteralKind::Integer) => {
                    let text = self.span(expression.span_id)?;
                    values.push(self.integer(text, false)?);
                }
                // A module constant is closed too: the number it evaluated to.
                Expr::Name(name) => {
                    self.token(expression.span_id, name)?;
                    let value = self
                        .constant_value(name)?
                        .ok_or(SourceCheckError::UnknownName)?;
                    values.push(value);
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        assert_eq!(values.len(), 1);
        values.pop().ok_or(SourceCheckError::Malformed)
    }

    fn array_constant_binary(
        &mut self,
        operator: &Spanned<BinaryOperator>,
        left: I256,
        right: I256,
    ) -> Result<I256> {
        let (token, value) = match operator.value {
            BinaryOperator::Add => ("+", left.checked_add(right)),
            BinaryOperator::Subtract => ("-", left.checked_sub(right)),
            BinaryOperator::Multiply => ("*", left.checked_mul(right)),
            BinaryOperator::Divide => ("/", left.checked_div(right)),
            BinaryOperator::Remainder => ("%", left.checked_rem(right)),
            _ => return Err(SourceCheckError::Unsupported),
        };
        self.token(operator.span_id, token)?;
        value.ok_or(SourceCheckError::Overflow)
    }

    pub(super) fn array_tasks(
        &mut self,
        expression: &'a Expression,
        array: &'a ArrayExpression,
        expected: Option<SourceType>,
        visited: bool,
        tasks: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        self.array_syntax(expression, array)?;
        if array.labels.iter().any(Option::is_some) {
            return self.labeled_tasks(expression, array, expected, visited, tasks, values);
        }
        if array.length.is_some() {
            return self.repeat_tasks(expression, array, expected, visited, tasks, values);
        }
        let element = match expected {
            Some(ty) => {
                let (element, extent) = self.array_layout(ty)?;
                if extent as usize != array.elements.len() {
                    return Err(SourceCheckError::Type);
                }
                Some(element)
            }
            None => None,
        };
        if visited {
            if values.len() < array.elements.len() {
                return Err(SourceCheckError::Malformed);
            }
            let children = values.split_off(values.len() - array.elements.len());
            values.push(self.array_pack(children, element)?);
        } else {
            self.charge(array.elements.len() + 1)?;
            tasks.push(EvalTask::Expression(expression, expected, true));
            for child in array.elements.iter().rev() {
                tasks.push(EvalTask::Expression(child, element, false));
            }
        }
        Ok(())
    }

    fn array_pack(
        &mut self,
        mut children: Vec<Value>,
        expected: Option<SourceType>,
    ) -> Result<Value> {
        let element = match expected {
            Some(ty) => ty,
            None => self.array_infer_element(&children)?,
        };
        let extent = u32::try_from(children.len()).map_err(|_| SourceCheckError::Budget)?;
        self.charge(children.len())?;
        for child in &mut children {
            *child = self.view_as(*child, element)?;
            self.release(*child)?;
        }
        let ty = self.array_type(element, extent)?;
        let value = self.fresh_value(ty)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        self.charge(children.len())?;
        self.proof.owners[owner].slots = (0..extent).collect();
        self.proof.owners[owner].children = children;
        self.proof.owners[owner].materialized = true;
        self.obligate(owner);
        Ok(value)
    }

    pub(super) fn array_infer_element(&mut self, children: &[Value]) -> Result<SourceType> {
        let first = children.first().ok_or(SourceCheckError::Type)?;
        let mut ty = array_element_view(*first);
        self.charge(children.len())?;
        for child in children {
            let actual = array_element_view(*child);
            if let (
                SourceType::Integer { lower, upper },
                SourceType::Integer { lower: a, upper: b },
            ) = (ty, actual)
            {
                ty = SourceType::Integer {
                    lower: lower.min(a),
                    upper: upper.max(b),
                };
            } else {
                self.compatible(ty, actual)?;
            }
        }
        Ok(ty)
    }

    /// Original slots are append-only in meaning, even after their ownership moved.
    pub(super) fn array_child(&mut self, value: Value, slot: u32) -> Result<Value> {
        let element = self.position_type(value.ty, slot)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        self.charge(self.proof.owners[owner].slots.len() + 1)?;
        let child = match self.proof.owners[owner].slots.binary_search(&slot) {
            Ok(index) => self.proof.owners[owner].children[index],
            Err(index) => {
                let child = self.input_value(element)?;
                self.proof.owners[owner].slots.insert(index, slot);
                self.proof.owners[owner].children.insert(index, child);
                child
            }
        };
        if let Some(ty) = value.structural {
            assert_eq!(self.position_count(value.ty)?, self.position_count(ty)?);
            let declared = self.position_type(ty, slot)?;
            self.view_as(child, declared)
        } else {
            Ok(child)
        }
    }

    pub(super) fn array_select(
        &mut self,
        expression: &'a Expression,
        select: &'a SelectExpression,
        index: Value,
    ) -> Result<Value> {
        self.child(expression.span_id, select.base.span_id)?;
        self.child(expression.span_id, select.index.span_id)?;
        for (span, token) in [(select.dot, "."), (select.open, "["), (select.close, "]")] {
            self.child(expression.span_id, span)?;
            self.token(span, token)?;
        }
        if select.length.is_some() {
            return Err(SourceCheckError::Unsupported);
        }
        // An open domain is a number: `storage.[position]` with `position InstructionOffset`.
        let index = self.open_number(index)?;
        let index = self.scalar_facts(index)?;
        self.scalar(index)?;
        // A variable index, or a base that is a field path, is a bounds-proved read of a
        // borrowed or sliced array (`extents.rs`); owned fixed arrays keep their exact slots.
        if index.lower != index.upper || !matches!(select.base.value, Expr::Name(_)) {
            return self.dynamic_select(select, index);
        }
        self.position_select(&select.base, index.lower)
    }

    pub(super) fn position_select(&mut self, base: &'a Expression, index: I256) -> Result<Value> {
        let value = self.array_local(base)?;
        let extent = self.position_count(value.ty)?;
        if index < I256::ZERO || index >= I256::from(extent) {
            return Err(SourceCheckError::Type);
        }
        let slot = index.as_u32();
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        self.charge(self.proof.owners[owner].moved.len() + 1)?;
        let position = self.proof.owners[owner].moved.binary_search(&slot);
        if position.is_ok() {
            return Err(SourceCheckError::UseAfterMove);
        }
        let child = self.array_child(value, slot)?;
        if child.owner.is_some() {
            self.charge_flow()?;
            self.proof.owners[owner]
                .moved
                .insert(position.unwrap_err(), slot);
            if self.proof.owners[owner].moved.len() == extent as usize {
                assert!(extent > 0);
                self.proof.owners[owner].drained = true;
                self.proof.owners[owner].live = false;
            }
        }
        self.scalar_facts(child)
    }

    pub(super) fn array_local(&mut self, root: &'a Expression) -> Result<Value> {
        let mut expression = root;
        loop {
            self.charge(1)?;
            if let Expr::Group(child) = &expression.value {
                self.group_syntax(expression, child)?;
                expression = child;
                continue;
            }
            let Expr::Name(name) = &expression.value else {
                return Err(SourceCheckError::Unsupported);
            };
            self.token(expression.span_id, name)?;
            let index = self
                .local_position(name)?
                .ok_or(SourceCheckError::UnknownName)?;
            let value = self.proof.names[index].1;
            if !value.available {
                return Err(SourceCheckError::UseAfterMove);
            }
            self.position_count(value.ty)?;
            return Ok(value);
        }
    }

    pub(super) fn array_complete(&mut self, value: Value) -> Result<()> {
        if matches!(
            value.ty,
            SourceType::Array { .. } | SourceType::Sequence { .. }
        ) {
            let owner = value.owner.ok_or(SourceCheckError::Type)?;
            self.charge(1)?;
            if !self.proof.owners[owner].moved.is_empty() || self.proof.owners[owner].drained {
                return Err(SourceCheckError::UseAfterMove);
            }
        }
        Ok(())
    }
}

pub(super) fn array_element_view(value: Value) -> SourceType {
    if let Some(id) = value.sum {
        SourceType::Sum { id }
    } else if let Some(ty) = value.structural {
        ty
    } else if let Some((lower, upper)) = value.integer_domain {
        assert!(matches!(value.ty, SourceType::Integer { .. }));
        SourceType::Integer { lower, upper }
    } else {
        value.ty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(text: &str) -> Result<CheckedSourceModule> {
        let source = parser::parse_strict(text).unwrap();
        check_module("arrays", &source, &[], crate::Limits::default())
    }

    #[test]
    fn opaque_arrays_and_selected_slots_have_extent_independent_original_origins() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        let ty = checker
            .array_type(SourceType::Generic { index: 0 }, u32::MAX)
            .unwrap();
        let value = checker.input_value(ty).unwrap();
        checker.proof.names.push(("values", value));
        let inputs = checker.summary_inputs().unwrap();
        let summary = checker.summarize(&inputs, Some(value)).unwrap();
        assert_eq!(summary.inputs.len(), 1);
        assert!(summary.inputs[0].children.is_empty());
        assert_eq!(checker.proof.owners.len(), 1);
        let child = checker.array_child(value, 123456).unwrap();
        let repeated = checker.array_child(value, 123456).unwrap();
        assert_eq!(child.identity, repeated.identity);
        assert_eq!(child.owner, repeated.owner);
        assert_eq!(checker.proof.owners.len(), 2);
        let inputs = checker.summary_inputs().unwrap();
        assert_eq!(inputs.nodes.len(), 2);
        assert_eq!(inputs.nodes[0].slots, [123456]);
        assert!(inputs.nodes[1].children.is_empty());
    }

    #[test]
    fn scalar_indexing_copies_identity_and_does_not_end_the_array() {
        let text = concat!(
            "f() in -16 to 16:\n",
            "  values: [1, 2]\n",
            "  first: values.[0]\n",
            "  again: values.[0]\n",
            "  rest: values\n",
            "  return -again\n",
        );
        assert_eq!(check(text).unwrap_err(), SourceCheckError::Unconsumed);
        let source = parser::parse_strict("values: [1, 2]\n").unwrap();
        let Item::Binding(binding) = &source.items[0].value else {
            unreachable!()
        };
        let mut checker = Checker::new(&source, crate::Limits::default());
        let value = checker
            .evaluate(&binding.value.as_ref().unwrap().expression, None)
            .unwrap();
        let first = checker.array_child(value, 0).unwrap();
        let again = checker.array_child(value, 0).unwrap();
        assert_eq!(first.identity, again.identity);
        assert_eq!(first.integer_domain, Some((I256::ONE, I256::from(2))));
        assert!(!checker.proof.owners[value.owner.unwrap()].drained);
        assert!(checker.proof.owners[value.owner.unwrap()].moved.is_empty());
    }

    #[test]
    fn zero_and_scalar_arrays_are_not_implicit_end_operations() {
        assert!(check("f(element Type) [element; 0]: return []\n").is_ok());
        assert!(
            check("f(element Type, values [element; 0]) [element; 0]: return values\n").is_ok()
        );
        assert_eq!(
            check("f(element Type, values [element; 0]): return\n").unwrap_err(),
            SourceCheckError::Unconsumed
        );
        assert_eq!(
            check("f():\n  values: [1]\n  x: values.[0]\n  return\n").unwrap_err(),
            SourceCheckError::Unconsumed
        );
    }

    #[test]
    fn scalar_selections_preserve_the_enclosing_unary_evaluation() {
        let source = parser::parse_strict("f(): return -values.[0]\n").unwrap();
        let Item::Binding(binding) = &source.items[0].value else {
            unreachable!()
        };
        let Expr::Block(block) = &binding.value.as_ref().unwrap().expression.value else {
            unreachable!()
        };
        let Statement::Return(ret) = &block.statements[0].value else {
            unreachable!()
        };
        let mut checker = Checker::new(&source, crate::Limits::default());
        let ty = checker
            .array_type(Value::integer(I256::ONE, I256::from(2)).ty, 1)
            .unwrap();
        let array = checker.input_value(ty).unwrap();
        checker.proof.names.push(("values", array));
        let result = checker.expression(ret.value.as_ref().unwrap()).unwrap();
        assert_eq!(result.lower, I256::from(-2));
        assert_eq!(result.upper, -I256::ONE);
    }

    fn deep_index_expression() -> (SourceFile, Expression) {
        let mut source = parser::parse_strict("").unwrap();
        let depth = 1024;
        let mut text = String::from("-");
        for _ in 0..depth {
            text.push_str("(indices.[");
        }
        let literal = text.len();
        text.push('0');
        for _ in 0..depth {
            text.push_str("])");
        }
        source.text = text.into();
        let end = source.text.len();
        let mut deep = Spanned::new(
            Expr::Literal(LiteralKind::Integer),
            source.span_table.insert_range(literal..literal + 1),
        );
        for level in (0..depth).rev() {
            let start = 1 + level * "(indices.[".len();
            let close = end - 2 - level * 2;
            let index = Spanned::new(
                Expr::Select(Box::new(SelectExpression {
                    base: Spanned::new(
                        Expr::Name("indices".to_owned()),
                        source.span_table.insert_range(start + 1..start + 8),
                    ),
                    dot: source.span_table.insert_range(start + 8..start + 9),
                    open: source.span_table.insert_range(start + 9..start + 10),
                    index: deep,
                    length: None,
                    close: source.span_table.insert_range(close..close + 1),
                })),
                source.span_table.insert_range(start + 1..close + 1),
            );
            deep = Spanned::new(
                Expr::Group(Box::new(index)),
                source.span_table.insert_range(start..close + 2),
            );
        }
        let root = Spanned::new(
            Expr::Unary {
                operator: Spanned::new(UnaryOperator::Negate, source.span_table.insert_range(0..1)),
                operand: Box::new(deep),
            },
            source.span_table.insert_range(0..end),
        );
        (source, root)
    }

    #[test]
    fn deep_manual_index_expressions_use_one_iterative_expression_stack() {
        let (source, root) = deep_index_expression();
        for bounded in [false, true] {
            let limits = crate::Limits {
                work_max: if bounded {
                    1000
                } else {
                    crate::Limits::default().work_max
                },
                ..crate::Limits::default()
            };
            let mut checker = Checker::new(&source, limits);
            let ty = checker
                .array_type(
                    SourceType::Integer {
                        lower: I256::ZERO,
                        upper: I256::ZERO,
                    },
                    1,
                )
                .unwrap();
            let values = checker.input_value(ty).unwrap();
            checker.proof.names.push(("indices", values));
            let result = checker.expression(&root);
            if bounded {
                assert!(matches!(result, Err(SourceCheckError::Budget)));
            } else {
                let value = result.unwrap();
                assert_eq!(value.lower, I256::ZERO);
                assert_eq!(value.upper, I256::ZERO);
            }
        }
        // Recursive source-AST Drop is not the evaluator being exercised here.
        let mut pending = vec![root];
        while let Some(expression) = pending.pop() {
            match expression.value {
                Expr::Group(child) | Expr::Unary { operand: child, .. } => pending.push(*child),
                Expr::Select(select) => {
                    pending.push(select.base);
                    pending.push(select.index);
                }
                Expr::Name(_) | Expr::Literal(_) => {}
                _ => unreachable!(),
            }
        }
    }

    #[test]
    fn array_inference_keeps_scalar_coverage_separate_from_current_facts() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        let scalar = checker
            .fresh_value(SourceType::Integer {
                lower: I256::ZERO,
                upper: I256::ONE,
            })
            .unwrap();
        checker.guard_integer(scalar, I256::ZERO).unwrap();
        let narrowed = checker.scalar_facts(scalar).unwrap();
        assert_eq!(narrowed.lower, I256::ZERO);
        assert_eq!(narrowed.upper, I256::ZERO);
        let array = checker.array_pack(vec![narrowed], None).unwrap();
        let selected = checker.array_child(array, 0).unwrap();
        assert_eq!(selected.identity, scalar.identity);
        assert_eq!(selected.integer_domain, Some((I256::ZERO, I256::ONE)));
        assert_eq!(selected.lower, I256::ZERO);
        assert_eq!(selected.upper, I256::ZERO);
    }

    #[test]
    fn imported_array_views_keep_recursive_sum_coverage_without_new_payloads() {
        // The declared sum's coverage rides through an imported array view: dropping an arm of a
        // `Big` taken out of an imported `[Big; 1]` is `Type`, covering it is accepted.
        for nested in [false, true] {
            let (big, literal, extraction) = if nested {
                (
                    "[[Big; 1]; 1]",
                    "[[A]]",
                    "  inner: values.[0]\n  value: inner.[0]\n",
                )
            } else {
                ("[Big; 1]", "[A]", "  value: values.[0]\n")
            };
            let library_text = format!(
                "Big is A or B or C\nFlag is Seen\n\
                 Box(element) has value element\n\
                 make() {big}: return {literal}\n"
            );
            let library_source = parser::parse_strict(&library_text).unwrap();
            let library =
                check_module("library", &library_source, &[], crate::Limits::default()).unwrap();
            let bindings = concat!(
                "  wide: make()\n",
                "  boxed: Box(value: wide)\n",
                "  Box(values) := boxed\n",
            );
            let client = format!(
                "use library Big, Flag, Box, make\nobserve() Flag:\n\
                 {bindings}{extraction}  seen Flag: when value is\n    A then Seen\n    B then Seen\n"
            );
            let source = parser::parse_strict(&format!("{client}  return seen\n")).unwrap();
            assert_eq!(
                check_module("client", &source, &[&library], crate::Limits::default()).unwrap_err(),
                SourceCheckError::Type
            );
            let source = parser::parse_strict(&format!("{client}    C then Seen\n  return seen\n")).unwrap();
            let module =
                check_module("client", &source, &[&library], crate::Limits::default()).unwrap();
            assert_eq!(module.summaries("observe").len(), 1);
        }
    }

    #[test]
    fn array_work_and_flow_limits_have_exact_fail_closed_boundaries() {
        let source = parser::parse_strict(concat!(
            "swap(element Type, values [element; 2]) [element; 2]:\n",
            "  first: values.[0]\n",
            "  second: values.[1]\n",
            "  return [second, first]\n",
        ))
        .unwrap();
        for flow in [false, true] {
            let mut lower = 0;
            let mut upper = if flow {
                crate::Limits::default().flow_work_max
            } else {
                crate::Limits::default().work_max
            };
            for _ in 0..20 {
                if lower == upper {
                    break;
                }
                let middle = lower + (upper - lower) / 2;
                let mut limits = crate::Limits::default();
                if flow {
                    limits.flow_work_max = middle;
                } else {
                    limits.work_max = middle;
                }
                match check_file(&source, limits) {
                    Ok(_) => upper = middle,
                    Err(SourceCheckError::Budget) => lower = middle + 1,
                    other => panic!("{other:?}"),
                }
            }
            assert_eq!(lower, upper);
            assert!(lower > 0);
            let mut limits = crate::Limits::default();
            if flow {
                limits.flow_work_max = lower - 1;
            } else {
                limits.work_max = lower - 1;
            }
            assert_eq!(check_file(&source, limits), Err(SourceCheckError::Budget));
        }
    }

    #[test]
    fn fixed_lengths_are_closed_checked_constants_not_runtime_domains() {
        assert!(check("f(element Type, a [element; 1 + 1]) [element; 2]: return a\n").is_ok());
        // `size in 2 to 2` is a number parameter (annotated by a range, as `capacity Length` is),
        // so `[element; size]` is an array of a length to be found at each call, not the closed
        // length 2: the types differ (`Type`). It used to be `Unsupported` before number
        // parameters existed; either way a runtime domain is not a length.
        assert_eq!(
            check("f(element Type, size in 2 to 2, a [element; size]) [element; 2]: return a\n")
                .unwrap_err(),
            SourceCheckError::Type
        );
        assert_eq!(
            check("f(element Type, a [element; -1]) [element; 2]: return a\n").unwrap_err(),
            SourceCheckError::Type
        );
    }
}
