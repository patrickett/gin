//! Source-shaped validation before field annotations can become layout authority.
use super::*;

impl<'a> Checker<'a> {
    pub(super) fn syntax_parts(&mut self, parent: SpanId, parts: &[SpanId]) -> Result<()> {
        self.span(parent)?;
        self.charge(parts.len())?;
        let range = self.range_of(parent);
        let mut cursor = range.start;
        for part in parts {
            self.child(parent, *part)?;
            let part = self.range_of(*part);
            self.syntax_trivia(part.file, cursor, part.start)?;
            cursor = part.end;
        }
        self.syntax_trivia(range.file, cursor, range.end)
    }

    pub(super) fn schema_annotation_syntax(
        &mut self,
        annotation: &'a Spanned<Annotation>,
    ) -> Result<()> {
        match &annotation.value {
            Annotation::Expression(expression) => {
                self.syntax_parts(annotation.span_id, &[expression.span_id])?;
                self.schema_expression_syntax(expression)
            }
            Annotation::Domain {
                is_keyword: None,
                domain,
            } => {
                self.syntax_parts(annotation.span_id, &[domain.span_id])?;
                self.schema_domain_syntax(domain)
            }
            // `capacity Count(x) and capacity <= limit`: a field with claims.
            Annotation::Refined {
                base: Some(base),
                refinements,
            } => {
                let mut parts = vec![base.span_id];
                parts.extend(refinements.iter().map(|clause| clause.span_id));
                self.syntax_parts(annotation.span_id, &parts)?;
                self.schema_expression_syntax(base)?;
                self.schema_refinements_syntax(refinements)
            }
            _ => Err(SourceCheckError::Unsupported),
        }
    }

    fn schema_domain_syntax(&mut self, domain: &'a Spanned<TypeDomain>) -> Result<()> {
        self.charge(domain.refinements.len() + 2)?;
        let mut parts = Vec::new();
        if let Some(keyword) = domain.in_keyword {
            self.keyword(keyword, "in")?;
            parts.push(keyword);
        }
        parts.push(domain.base.span_id);
        parts.extend(domain.refinements.iter().map(|clause| clause.span_id));
        self.syntax_parts(domain.span_id, &parts)?;
        let DomainBase::Range {
            lower,
            to_keyword,
            upper,
        } = &domain.base.value
        else {
            return Err(SourceCheckError::Unsupported);
        };
        self.keyword(*to_keyword, "to")?;
        self.syntax_parts(
            domain.base.span_id,
            &[lower.span_id, *to_keyword, upper.span_id],
        )?;
        self.schema_expression_syntax(lower)?;
        self.schema_expression_syntax(upper)?;
        self.schema_refinements_syntax(&domain.refinements)?;
        Ok(())
    }

    fn schema_refinements_syntax(&mut self, refinements: &'a [Spanned<Refinement>]) -> Result<()> {
        for clause in refinements {
            self.keyword(clause.and_keyword, "and")?;
            self.syntax_parts(
                clause.span_id,
                &[clause.and_keyword, clause.predicate.span_id],
            )?;
            match &clause.predicate.value {
                RefinementPredicate::Expression(expression) => {
                    self.syntax_parts(clause.predicate.span_id, &[expression.span_id])?;
                    self.schema_expression_syntax(expression)?;
                }
                RefinementPredicate::ImplicitComparison { operator, right } => {
                    self.comparison(operator)?;
                    self.syntax_parts(
                        clause.predicate.span_id,
                        &[operator.span_id, right.span_id],
                    )?;
                    self.schema_expression_syntax(right)?;
                }
                RefinementPredicate::ImplicitDomain(domain) => {
                    self.syntax_parts(clause.predicate.span_id, &[domain.span_id])?;
                    self.schema_domain_syntax(domain)?;
                }
            }
        }
        Ok(())
    }

    fn schema_expression_syntax(&mut self, root: &'a Expression) -> Result<()> {
        self.charge(1)?;
        let mut pending = vec![root];
        while let Some(expression) = pending.pop() {
            self.charge(1)?;
            self.span(expression.span_id)?;
            match &expression.value {
                Expr::Name(name) => self.token(expression.span_id, name)?,
                Expr::Literal(LiteralKind::Integer) => {
                    let text = self.span(expression.span_id)?;
                    self.integer(text, false)?;
                }
                Expr::Group(child) => {
                    self.group_syntax(expression, child)?;
                    self.charge(1)?;
                    pending.push(child);
                }
                Expr::PrefixedType(prefix) if prefix.keyword.value == TypePrefixKind::Pointer => {
                    self.pointer_syntax(expression, prefix)?;
                    self.charge(1)?;
                    pending.push(&prefix.target);
                }
                // `data ref T` / `data mut T`: a field that borrows. Where one may appear is
                // decided when the field's type is resolved, not by its spelling.
                Expr::PrefixedType(prefix) if Self::is_borrow_prefix(prefix) => {
                    self.borrow_prefix_syntax(expression, prefix)?;
                    self.charge(1)?;
                    pending.push(&prefix.target);
                }
                Expr::Sequence(sequence) => {
                    self.sequence_syntax(expression, sequence)?;
                    self.charge(sequence.elements.len())?;
                    pending.extend(&sequence.elements);
                }
                Expr::Array(array) => self.schema_array_syntax(expression, array, &mut pending)?,
                Expr::Call { callee, arguments } => {
                    self.syntax_parts(expression.span_id, &[callee.span_id, arguments.span_id])?;
                    self.schema_arguments_syntax(arguments, &mut pending)?;
                    self.charge(1)?;
                    pending.push(callee);
                }
                Expr::Unary { operator, operand } => {
                    self.token(operator.span_id, "-")?;
                    self.syntax_parts(expression.span_id, &[operator.span_id, operand.span_id])?;
                    if matches!(operand.value, Expr::Literal(LiteralKind::Integer)) {
                        let text = self.span(operand.span_id)?;
                        self.integer(text, true)?;
                    } else {
                        self.charge(1)?;
                        pending.push(operand);
                    }
                }
                Expr::Binary {
                    left,
                    operator,
                    right,
                } => {
                    self.schema_operator_syntax(operator)?;
                    self.syntax_parts(
                        expression.span_id,
                        &[left.span_id, operator.span_id, right.span_id],
                    )?;
                    self.charge(2)?;
                    pending.extend([left.as_ref(), right.as_ref()]);
                }
                // `cell.field` or `cell.accessor`: a field of another field of the record, which
                // the claim's term resolution reads (an accessor unfolds to its field).
                Expr::Member { base, name } => {
                    self.child(expression.span_id, base.span_id)?;
                    self.child(expression.span_id, name.span_id)?;
                    self.name(name)?;
                    self.charge(1)?;
                    pending.push(base);
                }
                // `#size`, `#alignment`: a compile-time query in a claim.
                Expr::Query { hash, name } => {
                    self.child(expression.span_id, *hash)?;
                    self.child(expression.span_id, name.span_id)?;
                    self.token(*hash, "#")?;
                    self.name(name)?;
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        Ok(())
    }

    fn schema_operator_syntax(&mut self, operator: &Spanned<BinaryOperator>) -> Result<()> {
        let text = match operator.value {
            BinaryOperator::Add => "+",
            BinaryOperator::Subtract => "-",
            BinaryOperator::Multiply => "*",
            BinaryOperator::Divide => "/",
            BinaryOperator::Remainder => "%",
            BinaryOperator::Equal => "=",
            BinaryOperator::Less => "<",
            BinaryOperator::LessOrEqual => "<=",
            BinaryOperator::Greater => ">",
            BinaryOperator::GreaterOrEqual => ">=",
            BinaryOperator::BitAnd => "&",
            BinaryOperator::BitOr => "|",
            BinaryOperator::BitXor => "^",
            BinaryOperator::ShiftLeft => "<<",
            BinaryOperator::ShiftRight => ">>",
            BinaryOperator::And => return self.keyword(operator.span_id, "and"),
            BinaryOperator::Or => return self.keyword(operator.span_id, "or"),
        };
        self.token(operator.span_id, text)
    }

    fn schema_arguments_syntax(
        &mut self,
        arguments: &'a Spanned<Vec<Spanned<Argument>>>,
        pending: &mut Vec<&'a Expression>,
    ) -> Result<()> {
        let text = self.span(arguments.span_id)?;
        if !text.starts_with('(') || !text.ends_with(')') {
            return Err(SourceCheckError::Malformed);
        }
        let range = self.range_of(arguments.span_id);
        let mut cursor = range.start + 1;
        self.charge(arguments.value.len())?;
        for (index, argument) in arguments.value.iter().enumerate() {
            self.child(arguments.span_id, argument.span_id)?;
            let part = self.range_of(argument.span_id);
            if index == 0 {
                self.syntax_trivia(part.file, cursor, part.start)?;
            } else {
                self.schema_comma_gap(part.file, cursor, part.start, true)?;
            }
            let Argument::Positional(expression) = &argument.value else {
                return Err(SourceCheckError::Type);
            };
            self.syntax_parts(argument.span_id, &[expression.span_id])?;
            pending.push(expression);
            cursor = part.end;
        }
        if arguments.value.is_empty() {
            self.syntax_trivia(range.file, cursor, range.end - 1)
        } else {
            self.schema_comma_gap(range.file, cursor, range.end - 1, false)
        }
    }

    fn schema_array_syntax(
        &mut self,
        expression: &'a Expression,
        array: &'a ArrayExpression,
        pending: &mut Vec<&'a Expression>,
    ) -> Result<()> {
        let count = array.elements.len();
        let separators = array.separators.len();
        if (count == 0 && separators != 0)
            || (count > 0 && separators != count - 1 && separators != count)
        {
            return Err(SourceCheckError::Malformed);
        }
        self.charge_product(count, 2)?;
        self.charge(separators + 4)?;
        self.token(array.open, "[")?;
        self.token(array.close, "]")?;
        let mut parts = vec![array.open];
        for (index, element) in array.elements.iter().enumerate() {
            parts.push(element.span_id);
            pending.push(element);
            if let Some(separator) = array.separators.get(index) {
                self.token(*separator, ",")?;
                parts.push(*separator);
            }
        }
        if let Some(length) = &array.length {
            self.token(length.separator, ";")?;
            parts.extend([length.separator, length.value.span_id]);
            pending.push(&length.value);
        }
        parts.push(array.close);
        self.syntax_parts(expression.span_id, &parts)
    }

    /// Calls do not retain comma spans. Recognize only one punctuation token in a
    /// bounded lexer-trivia gap; comment text cannot masquerade as that token.
    fn schema_comma_gap(
        &mut self,
        file: u32,
        start: usize,
        end: usize,
        required: bool,
    ) -> Result<()> {
        let text = self.gap_text(file, start, end)?;
        self.charge(text.len())?;
        let bytes = text.as_bytes();
        let mut index = 0;
        let mut comment = false;
        let mut comma = false;
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
                    b',' if !comma => {
                        comma = true;
                        index += 1;
                    }
                    _ => return Err(SourceCheckError::Malformed),
                }
            }
        }
        if comment || (required && !comma) {
            return Err(SourceCheckError::Malformed);
        }
        Ok(())
    }
}
