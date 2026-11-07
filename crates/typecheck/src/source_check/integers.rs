//! Finite literal cases and identity-keyed, body-derived scalar guards.
use super::*;

impl<'a> Checker<'a> {
    /// The exact domain of `left <operator> right`. A result outside the checker's
    /// integer width is an overflow, never a wrap.
    pub(super) fn arithmetic(
        &mut self,
        operator: CheckedArithmetic,
        left: Value,
        right: Value,
    ) -> Result<Value> {
        let left = self.scalar_facts(left)?;
        let right = self.scalar_facts(right)?;
        self.scalar(left)?;
        self.scalar(right)?;
        self.charge(4)?;
        let (lower, upper) = eval::interval(
            operator,
            (left.lower, left.upper),
            (right.lower, right.upper),
        )
        .ok_or(SourceCheckError::Overflow)?;
        let mut result = Value::integer(lower, upper);
        // Adding or subtracting a constant keeps the exact relation to an input.
        let constant = |value: Value| (value.lower == value.upper).then_some(value.lower);
        result.affine = match operator {
            CheckedArithmetic::Add => match (left.affine, constant(left), right.affine, constant(right)) {
                (Some((base, offset)), _, _, Some(c)) | (_, Some(c), Some((base, offset)), _) => {
                    offset.checked_add(c).map(|offset| (base, offset))
                }
                _ => None,
            },
            CheckedArithmetic::Subtract => match (left.affine, constant(right), right.affine) {
                (Some((base, offset)), Some(c), _) => {
                    offset.checked_sub(c).map(|offset| (base, offset))
                }
                // Two numbers measured from the same input differ by a known constant.
                (Some((left_base, left_offset)), _, Some((right_base, right_offset)))
                    if left_base == right_base =>
                {
                    if let Some(difference) = left_offset.checked_sub(right_offset) {
                        result = Value::integer(difference, difference);
                    }
                    None
                }
                _ => None,
            },
            // The checker's own rules for these are `integer_operation`; this path takes `+ - *`.
            _ => None,
        };
        let result = self.relate_arithmetic(operator, left, right, result)?;
        Ok(result)
    }

    pub(super) fn scalar_facts(&mut self, mut value: Value) -> Result<Value> {
        if matches!(value.ty, SourceType::Integer { .. }) {
            self.charge(self.proof.scalars.len())?;
            if let Some(identity) = value.identity {
                let (_, lower, upper) = self
                    .proof
                    .scalars
                    .iter()
                    .find(|(id, _, _)| *id == identity)
                    .ok_or(SourceCheckError::Type)?;
                value.lower = value.lower.max(*lower);
                value.upper = value.upper.min(*upper);
            }
            if value.lower > value.upper {
                return Err(SourceCheckError::Type);
            }
            value.ty = SourceType::Integer {
                lower: value.lower,
                upper: value.upper,
            };
        }
        Ok(value)
    }

    pub(super) fn integer_pattern(
        &mut self,
        pattern: &'a Expression,
        span: SpanId,
    ) -> Result<I256> {
        self.child(span, pattern.span_id)?;
        match &pattern.value {
            Expr::Literal(LiteralKind::Integer) => {}
            Expr::Unary { operator, operand }
                if operator.value == UnaryOperator::Negate
                    && matches!(operand.value, Expr::Literal(LiteralKind::Integer)) => {}
            _ => return Err(SourceCheckError::Type),
        }
        let value = self.expression(pattern)?;
        assert!(value.identity.is_none());
        assert_eq!(value.lower, value.upper);
        Ok(value.lower)
    }

    pub(super) fn guard_integer(&mut self, value: Value, number: I256) -> Result<()> {
        let mut value = self.scalar_facts(value)?;
        self.scalar(value)?;
        if number < value.lower || number > value.upper {
            return Err(SourceCheckError::Type);
        }
        value.lower = number;
        value.upper = number;
        self.narrow_scalar(value)?;
        if let Some(identity) = value.identity {
            self.charge(self.proof.integer_guards.len() + 1)?;
            if !self.proof.integer_guards.contains(&(identity, number)) {
                self.proof.integer_guards.push((identity, number));
            }
        }
        Ok(())
    }

    pub(super) fn integer_match_tasks(
        &mut self,
        case: &'a MatchExpression,
        subject: Value,
        expected: Option<SourceType>,
        pending: &mut Vec<aggregates::EvalTask<'a>>,
    ) -> Result<()> {
        use aggregates::EvalTask;
        self.token(case.when_keyword, "when")?;
        self.token(case.is_keyword, "is")?;
        self.charge_product(case.arms.value.len(), 3)?;
        let subject = self.scalar_facts(subject)?;
        let (lower, upper) = subject.integer_domain.ok_or(SourceCheckError::Type)?;
        let mut numbers = Vec::new();
        for arm in &case.arms.value {
            self.child(case.arms.span_id, arm.span_id)?;
            self.child(arm.span_id, arm.pattern.span_id)?;
            self.child(arm.span_id, arm.then_keyword)?;
            self.child(arm.span_id, arm.body.span_id)?;
            self.token(arm.then_keyword, "then")?;
            let MatchPattern::Integer(pattern) = &arm.pattern.value else {
                return Err(SourceCheckError::Type);
            };
            let number = self.integer_pattern(pattern, arm.pattern.span_id)?;
            if number < lower || number > upper {
                return Err(SourceCheckError::Type);
            }
            numbers.push(number);
        }
        self.integer_coverage(&numbers, lower, upper)?;
        let mut feasible = Vec::new();
        for (index, number) in numbers.iter().enumerate() {
            if self.integer_guards_feasible(&[(subject, *number)])? {
                feasible.push(index);
            }
        }
        let selected = feasible[self.choose(feasible.len())?];
        pending.push(EvalTask::Arm(&case.arms.value[selected], subject, expected));
        self.charge_product(numbers.len(), feasible.len())?;
        for (index, arm) in case.arms.value.iter().enumerate() {
            // Every feasible arm is checked by exhaustive world replay. Only
            // impossible arms need an additional detached static check.
            if index != selected && !feasible.contains(&index) {
                pending.push(EvalTask::ValidateIntegerArm(
                    &arm.value,
                    subject,
                    numbers[index],
                    expected,
                ));
            }
        }
        Ok(())
    }

    fn integer_coverage(&mut self, numbers: &[I256], lower: I256, upper: I256) -> Result<()> {
        // Sorting costs depend on arm count, never on an I256 interval's width.
        self.charge_product(numbers.len(), numbers.len().max(1))?;
        let mut sorted = numbers.to_vec();
        sorted.sort_unstable();
        for pair in sorted.windows(2) {
            if pair[0] == pair[1] {
                return Err(SourceCheckError::Duplicate);
            }
        }
        if sorted.first() != Some(&lower) || sorted.last() != Some(&upper) {
            return Err(SourceCheckError::Type);
        }
        for pair in sorted.windows(2) {
            if pair[0].checked_add(I256::ONE) != Some(pair[1]) {
                return Err(SourceCheckError::Type);
            }
        }
        Ok(())
    }

    pub(super) fn integer_guards_feasible(&mut self, guards: &[(Value, I256)]) -> Result<bool> {
        self.charge(self.proof.scalars.len() + guards.len())?;
        let saved = self.proof.scalars.clone();
        let mut result = Ok(true);
        for (value, number) in guards {
            let narrowed = (|| {
                let mut value = self.scalar_facts(*value)?;
                if *number < value.lower || *number > value.upper {
                    return Err(SourceCheckError::Type);
                }
                value.lower = *number;
                value.upper = *number;
                self.narrow_scalar(value)
            })();
            match narrowed {
                Ok(()) => {}
                Err(SourceCheckError::Type) => {
                    result = Ok(false);
                    break;
                }
                Err(error) => {
                    result = Err(error);
                    break;
                }
            }
        }
        self.proof.scalars = saved;
        result
    }

    pub(super) fn hypothetical_integer(&mut self, mut value: Value, number: I256) -> Result<Value> {
        self.charge(
            self.proof.scalars.len()
                + self.proof.facts.len()
                + self.proof.integer_guards.len()
                + self.proof.names.len()
                + self.proof.owners.len(),
        )?;
        if let Some(identity) = value.identity {
            self.proof
                .facts
                .retain(|(left, _, right)| *left != identity && *right != identity);
            self.proof.integer_guards.retain(|(id, _)| *id != identity);
            let scalar = self
                .proof
                .scalars
                .iter_mut()
                .find(|(id, _, _)| *id == identity)
                .ok_or(SourceCheckError::Type)?;
            *scalar = (identity, number, number);
            for (_, alias) in &mut self.proof.names {
                if alias.identity == Some(identity) {
                    alias.lower = number;
                    alias.upper = number;
                }
            }
            for index in 0..self.proof.owners.len() {
                self.charge(self.proof.owners[index].children.len())?;
                for alias in &mut self.proof.owners[index].children {
                    if alias.identity == Some(identity) {
                        alias.lower = number;
                        alias.upper = number;
                    }
                }
            }
        }
        value.lower = number;
        value.upper = number;
        value.ty = SourceType::Integer {
            lower: number,
            upper: number,
        };
        Ok(value)
    }
}
