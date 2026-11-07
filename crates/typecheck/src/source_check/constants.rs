//! Module constants: `name := expression` with no parameters and no written type.
//!
//! A constant is an exact integer known at check time. It is evaluated with the exact
//! integer arithmetic of the checker (never wrapping), the first time it is needed or, at
//! the latest, once all names are declared, so a constant nobody uses is still checked. A
//! constant that depends on itself is an error that names it. The value is published with
//! the checked module, so an importer reads `core.slice.max_length` as the number it is.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Pending,
    /// Being evaluated: reaching it again is a cycle.
    Active,
    Done(I256),
}

struct Declared<'a> {
    binding: &'a Binding,
    state: State,
}

/// Constants nest through each other at most this deep, so evaluation cannot exhaust the stack.
const DEPTH_MAX: usize = 64;

#[derive(Default)]
pub(super) struct Constants<'a> {
    local: Vec<Declared<'a>>,
    imported: Vec<(String, I256)>,
    depth: usize,
}

/// Whether an item is a constant: a bare `name := expression`.
pub(super) fn is_constant(binding: &Binding) -> bool {
    binding.parameters.is_none()
        && binding.annotation.is_none()
        && binding.contract.is_none()
        && binding.convention.is_none()
        && binding.result_binder.is_none()
        && binding.alternatives.is_empty()
        && binding.extra_results.is_empty()
        && binding
            .value
            .as_ref()
            .is_some_and(|body| body.operator.value == BindingOperator::ColonEquals)
}

/// Whether the constant is an exact integer (literals, names, `+ - * / % << >>`,
/// `#target.register_bits`). Any other compile-time value (a record, text, a choice by target)
/// is a constant that is worked out by checking its expression as a function of no parameters.
pub(super) fn integer_form(root: &Expression) -> bool {
    match &root.value {
        Expr::Name(_) | Expr::Literal(LiteralKind::Integer) => true,
        Expr::Group(child) => integer_form(child),
        Expr::Unary { operator, operand } => {
            operator.value == UnaryOperator::Negate && integer_form(operand)
        }
        Expr::Binary { left, right, .. } => integer_form(left) && integer_form(right),
        Expr::Member { base, name } => {
            matches!(&base.value, Expr::Query { .. }) && name.value == "register_bits"
        }
        // `when #target.triple is 'aarch64-apple-darwin' then 72 else 36`: a number chosen by the
        // target, still an exact integer, so it can size an array or bound a domain.
        Expr::Conditional(case) => {
            target_choice(&case.condition).is_some()
                && integer_form(&case.then_branch)
                && integer_form(&case.else_branch)
        }
        _ => false,
    }
}

/// `#target.triple is 'text'`: the text, when the condition is that test.
fn target_choice(condition: &Expression) -> Option<&Expression> {
    let Expr::IsTest(test) = &condition.value else {
        return None;
    };
    let Expr::Member { base, name } = &test.subject.value else {
        return None;
    };
    let MatchPattern::Text(text) = &test.pattern.value else {
        return None;
    };
    (matches!(&base.value, Expr::Query { .. }) && name.value == "triple").then_some(text)
}

impl<'a> Checker<'a> {
    /// Declare a module-level constant. Its value is computed later, on first use.
    pub(super) fn declare_constant(&mut self, binding: &'a Binding, parent: SpanId) -> Result<()> {
        self.binding_spans(binding, parent)?;
        let name = binding.name.value.as_str();
        self.unique_symbol(name)?;
        self.constant_clash(name)?;
        self.charge(name.len() + 2)?;
        self.module.constants.local.push(Declared {
            binding,
            state: State::Pending,
        });
        Ok(())
    }

    fn constant_clash(&mut self, name: &str) -> Result<()> {
        let constants = &self.module.constants;
        let count = constants.local.len() + constants.imported.len();
        let clash = constants
            .local
            .iter()
            .any(|declared| declared.binding.name.value == name)
            || constants.imported.iter().any(|(other, _)| other == name);
        self.charge_product(count + 1, name.len() + 1)?;
        if clash {
            return Err(SourceCheckError::Duplicate);
        }
        Ok(())
    }

    /// Evaluate every constant of this module, so errors in an unused one are still found.
    pub(super) fn evaluate_constants(&mut self) -> diagnostics::DiagnosticResult<()> {
        for index in 0..self.module.constants.local.len() {
            let span = self.module.constants.local[index].binding.name.span_id;
            self.constant_at(index)
                .map_err(|error| self.failure(error, span))?;
        }
        Ok(())
    }

    /// The constants this module publishes, in declaration order.
    pub(super) fn exported_constants(&self) -> Vec<(String, I256)> {
        self.module
            .constants
            .local
            .iter()
            .filter_map(|declared| match declared.state {
                State::Done(value) => Some((declared.binding.name.value.clone(), value)),
                _ => None,
            })
            .collect()
    }

    /// Bring a dependency's constant into scope.
    pub(super) fn import_constant(
        &mut self,
        name: &str,
        dependency: &[(String, I256)],
    ) -> Result<()> {
        let value = dependency
            .iter()
            .find(|(other, _)| other == name)
            .map(|(_, value)| *value)
            .ok_or(SourceCheckError::UnknownName)?;
        self.constant_clash(name)?;
        self.module.constants.imported.push((name.to_owned(), value));
        Ok(())
    }

    /// Whether `name` is a constant and no local of the body being checked hides it.
    pub(super) fn constant_in_scope(&mut self, name: &str) -> Result<bool> {
        if self.module.constants.local.is_empty() && self.module.constants.imported.is_empty() {
            return Ok(false);
        }
        if self.local_position(name)?.is_some() {
            return Ok(false);
        }
        self.constant_exists(name)
    }

    pub(super) fn constant_exists(&mut self, name: &str) -> Result<bool> {
        let constants = &self.module.constants;
        if constants.local.is_empty() && constants.imported.is_empty() {
            return Ok(false);
        }
        let found = constants
            .local
            .iter()
            .any(|declared| declared.binding.name.value == name)
            || constants.imported.iter().any(|(other, _)| other == name);
        let count = constants.local.len() + constants.imported.len();
        self.charge_product(count + 1, name.len() + 1)?;
        Ok(found)
    }

    /// The value of constant `name`, evaluating it if it has not been yet.
    pub(super) fn constant_value(&mut self, name: &str) -> Result<Option<I256>> {
        let count = self.module.constants.local.len() + self.module.constants.imported.len();
        // A module with no constants pays nothing to learn that.
        if count == 0 {
            return Ok(None);
        }
        self.charge_product(count + 1, name.len() + 1)?;
        let constants = &self.module.constants;
        if let Some(index) = constants
            .local
            .iter()
            .position(|declared| declared.binding.name.value == name)
        {
            return self.constant_at(index).map(Some);
        }
        Ok(constants
            .imported
            .iter()
            .find(|(other, _)| other == name)
            .map(|(_, value)| *value))
    }

    /// The value of an already evaluated constant, without any work.
    pub(super) fn known_constant(&self, name: &str) -> Option<I256> {
        let constants = &self.module.constants;
        constants
            .local
            .iter()
            .find_map(|declared| match declared.state {
                State::Done(value) if declared.binding.name.value == name => Some(value),
                _ => None,
            })
            .or_else(|| {
                constants
                    .imported
                    .iter()
                    .find(|(other, _)| other == name)
                    .map(|(_, value)| *value)
            })
    }

    fn constant_at(&mut self, index: usize) -> Result<I256> {
        let declared = &self.module.constants.local[index];
        let binding = declared.binding;
        match declared.state {
            State::Done(value) => return Ok(value),
            State::Active => {
                let detail = format!("constant `{}` depends on itself", binding.name.value);
                return Err(self.explain(SourceCheckError::Malformed, detail));
            }
            State::Pending => {}
        }
        if self.module.constants.depth >= DEPTH_MAX {
            return Err(SourceCheckError::Budget);
        }
        self.module.constants.local[index].state = State::Active;
        self.module.constants.depth += 1;
        let body = binding.value.as_ref().ok_or(SourceCheckError::MissingBody)?;
        self.token(body.operator.span_id, ":=")?;
        let value = self.constant_expression(&body.expression);
        self.module.constants.depth -= 1;
        let value = value?;
        self.module.constants.local[index].state = State::Done(value);
        Ok(value)
    }

    /// The exact integer `root` denotes.
    pub(super) fn constant_expression(&mut self, root: &'a Expression) -> Result<I256> {
        self.charge(1)?;
        let mut tasks = vec![(root, false)];
        let mut values: Vec<I256> = Vec::new();
        while let Some((expression, visited)) = tasks.pop() {
            self.charge(1)?;
            let text = self.span(expression.span_id)?;
            match &expression.value {
                Expr::Name(name) => {
                    if text != name {
                        return Err(SourceCheckError::Malformed);
                    }
                    self.charge(name.len())?;
                    let value = self
                        .constant_value(name)?
                        .ok_or(SourceCheckError::UnknownName)?;
                    values.push(value);
                }
                Expr::Literal(LiteralKind::Integer) => values.push(self.integer(text, false)?),
                Expr::Group(child) => {
                    self.group_syntax(expression, child)?;
                    self.charge(1)?;
                    tasks.push((child, false));
                }
                Expr::Unary { operator, operand } if operator.value == UnaryOperator::Negate => {
                    self.child(expression.span_id, operator.span_id)?;
                    self.child(expression.span_id, operand.span_id)?;
                    self.token(operator.span_id, "-")?;
                    if visited {
                        let value = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(value.checked_neg().ok_or(SourceCheckError::Overflow)?);
                    } else {
                        self.charge(2)?;
                        tasks.push((expression, true));
                        tasks.push((operand, false));
                    }
                }
                Expr::Binary {
                    left,
                    operator,
                    right,
                } => {
                    self.child(expression.span_id, left.span_id)?;
                    self.child(expression.span_id, operator.span_id)?;
                    self.child(expression.span_id, right.span_id)?;
                    self.token(operator.span_id, binary_symbol(operator.value)?)?;
                    if visited {
                        let right = values.pop().ok_or(SourceCheckError::Malformed)?;
                        let left = values.pop().ok_or(SourceCheckError::Malformed)?;
                        values.push(self.constant_binary(operator.value, left, right)?);
                    } else {
                        self.charge(3)?;
                        tasks.push((expression, true));
                        tasks.push((right, false));
                        tasks.push((left, false));
                    }
                }
                // A number chosen by the target triple: only the branch the target takes is read.
                Expr::Conditional(case) if target_choice(&case.condition).is_some() => {
                    let text = target_choice(&case.condition).ok_or(SourceCheckError::Malformed)?;
                    self.child(expression.span_id, case.condition.span_id)?;
                    self.child(expression.span_id, case.then_branch.span_id)?;
                    self.child(expression.span_id, case.else_branch.span_id)?;
                    self.token(case.when_keyword, "when")?;
                    self.token(case.then_keyword, "then")?;
                    self.token(case.else_keyword, "else")?;
                    let written = self.span(text.span_id)?;
                    let wanted = quoted(written).ok_or(SourceCheckError::Malformed)?;
                    self.charge(wanted.len() + 1)?;
                    let taken = wanted == self.target.triple;
                    self.charge(1)?;
                    tasks.push(if taken { (&case.then_branch, false) } else { (&case.else_branch, false) });
                }
                // `#target.register_bits`: a fact about the machine, known now.
                Expr::Member { base, name }
                    if matches!(&base.value, Expr::Query { .. })
                        && name.value == "register_bits" =>
                {
                    let value = self.target_fact(expression, base, name)?;
                    values.push(value.lower);
                }
                _ => return Err(SourceCheckError::Unsupported),
            }
        }
        let value = values.pop().ok_or(SourceCheckError::Malformed)?;
        if !values.is_empty() {
            return Err(SourceCheckError::Malformed);
        }
        Ok(value)
    }

    fn constant_binary(&mut self, operator: BinaryOperator, left: I256, right: I256) -> Result<I256> {
        let overflow = SourceCheckError::Overflow;
        // The arithmetic itself is `eval::exact_binary`, shared with every other consumer.
        let result = eval::exact_binary(operator, left, right);
        // A shift by a usable amount has always cost one unit; the budget is unchanged.
        if matches!(operator, BinaryOperator::ShiftLeft | BinaryOperator::ShiftRight)
            && !matches!(result, eval::Exact::ShiftAmount(_))
        {
            self.charge(1)?;
        }
        match result {
            eval::Exact::Value(value) => Ok(value),
            eval::Exact::ShiftAmount(amount) => {
                let detail = format!("a shift by {amount} has no exact value");
                Err(self.explain(overflow, detail))
            }
            eval::Exact::Overflow => {
                let detail = format!("`{left} {} {right}` has no exact value here", binary_symbol(operator).unwrap_or("?"));
                Err(self.explain(overflow, detail))
            }
            eval::Exact::NotArithmetic => Err(SourceCheckError::Unsupported),
        }
    }
}

fn binary_symbol(operator: BinaryOperator) -> Result<&'static str> {
    Ok(match operator {
        BinaryOperator::Add => "+",
        BinaryOperator::Subtract => "-",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Divide => "/",
        BinaryOperator::Remainder => "%",
        BinaryOperator::ShiftLeft => "<<",
        BinaryOperator::ShiftRight => ">>",
        _ => return Err(SourceCheckError::Unsupported),
    })
}
