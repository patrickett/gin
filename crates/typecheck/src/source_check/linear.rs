//! Relations between numbers: `a + b <= c`, `n <= count`, `count - n >= 0`.
//!
//! Intervals alone cannot see that `count - n` is not negative when `n <= count`, or that
//! `first + n` fits when `first + count <= max` and `n <= count`. This module adds the missing
//! knowledge, as one general rule and no special case:
//!
//! * **Facts** are linear: every number has an identity (the immutable value it names), and a
//!   fact is `sum(c_i * x_i) <= k`. They come from three places: the order facts guards and
//!   claims already record (`facts`: `a < b`, `a <= b`, `a = b`), the interval of each identity
//!   (`scalars`), and **definitions** (`defs`): when `a + b` or `a - b` is computed and either
//!   side is already related to something, the result gets an identity `r` and the equation
//!   `r = a + b` is recorded.
//! * **Claims** become facts the same way: `first + count <= max` on a field is the definition
//!   of `r = first + count` and the interval `r <= max`. A claim that is destructured with its
//!   record keeps its identities, so it is still a fact about the same numbers afterwards; a
//!   record rebuilt from them shows its claim from the same facts.
//! * **Frame rule.** Facts are about identities, and an identity is an immutable number. Writing
//!   a place (`x :: e`) gives the place a new identity; the old facts stay true of the old
//!   number and say nothing of the place. A `mut` call changes only the places its `::`
//!   transitions name (and the places its body wrote, which are forgotten); every other place
//!   keeps its identity, hence its facts.
//! * **Soundness.** Definitions are timeless equalities between identities, order facts are
//!   rolled back with the branch that taught them, and definitions of identities created after
//!   a snapshot are dropped with it (identities are reused after a restore). The query is
//!   Fourier-Motzkin elimination over the rationals, which is sound for integers (an
//!   infeasible rational system has no integer solution); it is incomplete and gives up, with
//!   "unknown", when the component of related numbers is larger than `VARIABLES_MAX` or the
//!   rows outgrow `ROWS_MAX`.
use super::*;
use num_traits::ToPrimitive;

/// How many related numbers one query follows.
const VARIABLES_MAX: usize = 160;
/// How many inequalities elimination may hold at once.
const ROWS_MAX: usize = 4000;

/// `result = left + right` (`subtract` false) or `left - right` (true), or `left * right`
/// (`multiply`): a product is not linear unless one factor is a constant, so it gives elimination
/// no row for two numbers, but it is remembered, and the same product written again is the same
/// number (`count * width` in a claim is the `count * width` the caller computed).
#[derive(Clone, Copy, Debug)]
pub(super) struct Definition {
    pub result: u32,
    pub left: u32,
    pub right: Operand,
    pub subtract: bool,
    pub multiply: bool,
    /// `result = constant - left` (a constant minus a number): the two sum to the constant.
    pub reflect: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Operand {
    Identity(u32),
    Constant(i128),
}

/// `sum(coefficients[i] * x_i) <= bound`.
#[derive(Clone)]
struct Row {
    coefficients: Vec<i128>,
    bound: i128,
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

impl Row {
    fn normalize(&mut self) {
        let divisor = self.coefficients.iter().fold(0, |g, c| gcd(g, *c));
        if divisor > 1 {
            for c in &mut self.coefficients {
                *c /= divisor;
            }
            // Over the integers the bound can be rounded down.
            self.bound = self.bound.div_euclid(divisor);
        }
    }
}

impl<'a> Checker<'a> {
    /// Whether `identity` already takes part in a definition or an order fact.
    fn related(&mut self, identity: u32) -> Result<bool> {
        self.charge(self.proof.definitions.len() + self.proof.facts.len() + 1)?;
        Ok(self.proof.definitions.iter().any(|d| {
            d.result == identity
                || d.left == identity
                || matches!(d.right, Operand::Identity(r) if r == identity)
        }) || self
            .proof
            .facts
            .iter()
            .any(|(l, _, r)| *l == identity || *r == identity))
    }

    /// `result` is `left + right` or `left - right` (already worked out as an interval): when
    /// either operand is related to something, give the result an identity, record the
    /// equation, and tighten its interval with everything that is known.
    pub(super) fn relate_arithmetic(
        &mut self,
        operator: CheckedArithmetic,
        left: Value,
        right: Value,
        result: Value,
    ) -> Result<Value> {
        if operator == CheckedArithmetic::Multiply {
            return self.relate_product(left, right, result);
        }
        let subtract = match operator {
            CheckedArithmetic::Add => false,
            CheckedArithmetic::Subtract => true,
            _ => return Ok(result),
        };
        let constant = |value: Value| {
            (value.lower == value.upper)
                .then(|| value.lower.to_i128())
                .flatten()
        };
        let (first, second) = match (left.identity, right.identity, constant(left), constant(right)) {
            (Some(l), Some(r), _, _) => (l, Operand::Identity(r)),
            (Some(l), None, _, Some(c)) => (l, Operand::Constant(c)),
            // `c + x` is `x + c`; `c - x` is `x + result = c`, a reflected definition.
            (None, Some(r), Some(c), _) => (r, Operand::Constant(c)),
            _ => return Ok(result),
        };
        // `constant - x` is related at once: it is the shape of `length - used`, which a later
        // claim (`from + count <= length`) is stated over and met after the arguments are made.
        let touched = self.proof.relate_always
            || subtract
            || self.related(first)?
            || match second {
                Operand::Identity(r) => self.related(r)?,
                Operand::Constant(_) => false,
            };
        if !touched {
            return Ok(result);
        }
        let mut result = self.ensure_identity(result)?;
        let identity = result.identity.ok_or(SourceCheckError::Type)?;
        self.charge(1)?;
        self.proof.definitions.push(Definition {
            result: identity,
            left: first,
            right: second,
            subtract,
            multiply: false,
            reflect: subtract && left.identity.is_none(),
        });
        if let Some((lower, upper)) = self.linear_bounds(identity)? {
            let lower = lower.max(result.lower);
            let upper = upper.min(result.upper);
            // An inconsistent system proves nothing; keep what the interval said.
            if lower <= upper {
                result.lower = lower;
                result.upper = upper;
                result.ty = SourceType::Integer { lower, upper };
                self.narrow_scalar(result)?;
            }
        }
        Ok(result)
    }

    /// The rows describing the numbers related to `roots`, and those numbers. `None` when the
    /// component is too large, or a bound does not fit the elimination's integers.
    fn linear_system(&mut self, roots: &[u32], extra: usize) -> Result<Option<(Vec<u32>, Vec<Row>)>> {
        let mut variables: Vec<u32> = Vec::new();
        for root in roots {
            if !variables.contains(root) {
                variables.push(*root);
            }
        }
        loop {
            self.charge(self.proof.definitions.len() + self.proof.facts.len() + 1)?;
            let before = variables.len();
            let mut found: Vec<u32> = Vec::new();
            for d in &self.proof.definitions {
                let mut ids = vec![d.result, d.left];
                if let Operand::Identity(r) = d.right {
                    ids.push(r);
                }
                if ids.iter().any(|id| variables.contains(id)) {
                    found.extend(ids);
                }
            }
            for (l, _, r) in &self.proof.facts {
                if variables.contains(l) || variables.contains(r) {
                    found.extend([*l, *r]);
                }
            }
            for id in found {
                if !variables.contains(&id) {
                    variables.push(id);
                }
            }
            if variables.len() > VARIABLES_MAX {
                return Ok(None);
            }
            if variables.len() == before {
                break;
            }
        }
        let width = variables.len() + extra;
        let at = |id: u32| variables.iter().position(|v| *v == id);
        let mut rows: Vec<Row> = Vec::new();
        let row = |entries: &[(usize, i128)], bound: i128| {
            let mut coefficients = vec![0; width];
            for (index, c) in entries {
                coefficients[*index] += *c;
            }
            Row { coefficients, bound }
        };
        self.charge(variables.len() * 2)?;
        for (index, id) in variables.iter().enumerate() {
            if let Some((_, lower, upper)) = self.proof.scalars.iter().find(|(s, _, _)| s == id) {
                if let Some(upper) = upper.to_i128() {
                    rows.push(row(&[(index, 1)], upper));
                }
                if let Some(lower) = lower.to_i128() {
                    rows.push(row(&[(index, -1)], lower.checked_neg().ok_or(SourceCheckError::Overflow)?));
                }
            }
        }
        for d in &self.proof.definitions {
            let (Some(result), Some(left)) = (at(d.result), at(d.left)) else {
                continue;
            };
            if d.multiply {
                // A product gives elimination no row (a constant factor would be linear, but
                // that is not built); it is remembered so the same product is the same number.
                continue;
            }
            match d.right {
                Operand::Identity(r) => {
                    let Some(right) = at(r) else { continue };
                    let sign = if d.subtract { 1 } else { -1 };
                    // result - left -+ right = 0
                    rows.push(row(&[(result, 1), (left, -1), (right, sign)], 0));
                    rows.push(row(&[(result, -1), (left, 1), (right, -sign)], 0));
                }
                Operand::Constant(c) if d.reflect => {
                    // result + left = c
                    rows.push(row(&[(result, 1), (left, 1)], c));
                    rows.push(row(&[(result, -1), (left, -1)], c.checked_neg().ok_or(SourceCheckError::Overflow)?));
                }
                Operand::Constant(c) => {
                    let c = if d.subtract { c.checked_neg() } else { Some(c) }
                        .ok_or(SourceCheckError::Overflow)?;
                    // result - left = c
                    rows.push(row(&[(result, 1), (left, -1)], c));
                    rows.push(row(&[(result, -1), (left, 1)], c.checked_neg().ok_or(SourceCheckError::Overflow)?));
                }
            }
        }
        for (l, operator, r) in &self.proof.facts {
            let (Some(l), Some(r)) = (at(*l), at(*r)) else {
                continue;
            };
            match operator {
                BinaryOperator::Equal => {
                    rows.push(row(&[(l, 1), (r, -1)], 0));
                    rows.push(row(&[(l, -1), (r, 1)], 0));
                }
                BinaryOperator::Less => rows.push(row(&[(l, 1), (r, -1)], -1)),
                BinaryOperator::LessOrEqual => rows.push(row(&[(l, 1), (r, -1)], 0)),
                BinaryOperator::Greater => rows.push(row(&[(r, 1), (l, -1)], -1)),
                BinaryOperator::GreaterOrEqual => rows.push(row(&[(r, 1), (l, -1)], 0)),
                _ => {}
            }
        }
        Ok(Some((variables, rows)))
    }

    /// The tightest interval for variable `target` that the rows imply, by eliminating every
    /// other variable. `None` when elimination gave up or the rows contradict each other.
    fn project(&mut self, mut rows: Vec<Row>, target: usize) -> Result<Option<(Option<i128>, Option<i128>)>> {
        let width = rows.first().map_or(0, |row| row.coefficients.len());
        let mut remaining: Vec<usize> = (0..width).filter(|v| *v != target).collect();
        rows = Self::merge_rows(rows);
        rows = Self::substitute_equalities(rows, target, width);
        while !remaining.is_empty() {
            // The variable whose elimination adds the fewest rows goes first.
            let cost = |variable: usize| {
                let positive = rows.iter().filter(|r| r.coefficients[variable] > 0).count();
                let negative = rows.iter().filter(|r| r.coefficients[variable] < 0).count();
                positive * negative
            };
            let (position, _) = remaining
                .iter()
                .enumerate()
                .min_by_key(|(_, variable)| cost(**variable))
                .ok_or(SourceCheckError::Malformed)?;
            let variable = remaining.swap_remove(position);
            self.charge(rows.len() + 1)?;
            let mut positive = Vec::new();
            let mut negative = Vec::new();
            let mut rest = Vec::new();
            for row in rows {
                match row.coefficients[variable].signum() {
                    1 => positive.push(row),
                    -1 => negative.push(row),
                    _ => rest.push(row),
                }
            }
            if rest.len() + positive.len() * negative.len() > ROWS_MAX {
                return Ok(None);
            }
            self.charge(positive.len() * negative.len() + 1)?;
            for p in &positive {
                for n in &negative {
                    let (a, b) = (p.coefficients[variable], -n.coefficients[variable]);
                    let mut combined = Row {
                        coefficients: vec![0; width],
                        bound: 0,
                    };
                    for i in 0..width {
                        let value = p.coefficients[i]
                            .checked_mul(b)
                            .and_then(|x| n.coefficients[i].checked_mul(a).and_then(|y| x.checked_add(y)));
                        let Some(value) = value else { return Ok(None) };
                        combined.coefficients[i] = value;
                    }
                    let bound = p
                        .bound
                        .checked_mul(b)
                        .and_then(|x| n.bound.checked_mul(a).and_then(|y| x.checked_add(y)));
                    let Some(bound) = bound else { return Ok(None) };
                    combined.bound = bound;
                    combined.normalize();
                    rest.push(combined);
                }
            }
            rows = Self::merge_rows(rest);
        }
        let (mut lower, mut upper): (Option<i128>, Option<i128>) = (None, None);
        for row in &rows {
            let a = row.coefficients[target];
            if a == 0 {
                if row.bound < 0 {
                    return Ok(None);
                }
                continue;
            }
            if a > 0 {
                let high = row.bound.div_euclid(a);
                upper = Some(upper.map_or(high, |u| u.min(high)));
            } else {
                // a*t <= b with a < 0: t >= ceil(b / a) = -floor(b / -a).
                let low = -(row.bound.div_euclid(-a));
                lower = Some(lower.map_or(low, |l| l.max(low)));
            }
        }
        Ok(Some((lower, upper)))
    }

    /// Equalities (a row and its negation with opposite bounds) with a variable of coefficient
    /// one are used to replace that variable everywhere: the sums a claim is built from are
    /// equations, and eliminating them first keeps the inequalities from multiplying. A row that
    /// would overflow is left as it was.
    fn substitute_equalities(mut rows: Vec<Row>, target: usize, width: usize) -> Vec<Row> {
        loop {
            let mut found: Option<(usize, usize, usize)> = None;
            'search: for i in 0..rows.len() {
                for j in (i + 1)..rows.len() {
                    let (a, b) = (&rows[i], &rows[j]);
                    if a.bound.checked_add(b.bound) != Some(0) {
                        continue;
                    }
                    if !a.coefficients.iter().zip(&b.coefficients).all(|(x, y)| x.checked_neg() == Some(*y)) {
                        continue;
                    }
                    if let Some(v) = (0..width).find(|v| *v != target && a.coefficients[*v].abs() == 1) {
                        found = Some((i, j, v));
                        break 'search;
                    }
                }
            }
            let Some((i, j, v)) = found else {
                return rows;
            };
            let equation = rows[i].clone();
            let e_v = equation.coefficients[v];
            let mut next = Vec::with_capacity(rows.len());
            let mut overflow = false;
            for (index, row) in rows.iter().enumerate() {
                if index == i || index == j {
                    continue;
                }
                let c = row.coefficients[v];
                if c == 0 {
                    next.push(row.clone());
                    continue;
                }
                // row - (c / e_v) * equation, which removes `v`.
                let factor = c * e_v;
                let mut replaced = row.clone();
                for k in 0..width {
                    match equation.coefficients[k].checked_mul(factor).and_then(|x| row.coefficients[k].checked_sub(x)) {
                        Some(value) => replaced.coefficients[k] = value,
                        None => overflow = true,
                    }
                }
                match equation.bound.checked_mul(factor).and_then(|x| row.bound.checked_sub(x)) {
                    Some(bound) => replaced.bound = bound,
                    None => overflow = true,
                }
                next.push(replaced);
            }
            if overflow {
                return rows;
            }
            rows = Self::merge_rows(next);
        }
    }

    /// Rows with the same coefficients collapse to the one with the smallest bound.
    fn merge_rows(mut rows: Vec<Row>) -> Vec<Row> {
        rows.sort_by(|a, b| a.coefficients.cmp(&b.coefficients).then(a.bound.cmp(&b.bound)));
        rows.dedup_by(|later, earlier| later.coefficients == earlier.coefficients);
        rows
    }

    /// The interval the facts imply for the number `identity`, when they imply one.
    pub(super) fn linear_bounds(&mut self, identity: u32) -> Result<Option<(I256, I256)>> {
        let Some((variables, rows)) = self.linear_system(&[identity], 0)? else {
            return Ok(None);
        };
        let target = variables.iter().position(|v| *v == identity).ok_or(SourceCheckError::Type)?;
        Ok(match self.project(rows, target)? {
            Some((Some(lower), Some(upper))) => Some((I256::from(lower), I256::from(upper))),
            _ => None,
        })
    }

    /// Whether `left <operator> right` follows from the facts about the two numbers.
    pub(super) fn linear_entails(
        &mut self,
        operator: BinaryOperator,
        left: u32,
        right: u32,
    ) -> Result<bool> {
        let Some((variables, mut rows)) = self.linear_system(&[left, right], 1)? else {
            return Ok(false);
        };
        let (Some(l), Some(r)) = (
            variables.iter().position(|v| *v == left),
            variables.iter().position(|v| *v == right),
        ) else {
            return Ok(false);
        };
        let d = variables.len();
        // d = left - right
        let mut equal = vec![0; d + 1];
        equal[d] = 1;
        equal[l] -= 1;
        equal[r] += 1;
        let negated: Vec<i128> = equal.iter().map(|c| -c).collect();
        rows.push(Row { coefficients: equal, bound: 0 });
        rows.push(Row { coefficients: negated, bound: 0 });
        let Some((lower, upper)) = self.project(rows, d)? else {
            return Ok(false);
        };
        Ok(match operator {
            BinaryOperator::Less => upper.is_some_and(|u| u <= -1),
            BinaryOperator::LessOrEqual => upper.is_some_and(|u| u <= 0),
            BinaryOperator::Greater => lower.is_some_and(|l| l >= 1),
            BinaryOperator::GreaterOrEqual => lower.is_some_and(|l| l >= 0),
            BinaryOperator::Equal => lower == Some(0) && upper == Some(0),
            _ => false,
        })
    }
}

impl<'a> Checker<'a> {
    /// `left * right`: the same two numbers multiplied before give the same result identity.
    fn relate_product(&mut self, left: Value, right: Value, result: Value) -> Result<Value> {
        let constant = |value: Value| {
            (value.lower == value.upper)
                .then(|| value.lower.to_i128())
                .flatten()
        };
        let (first, second) = match (left.identity, right.identity, constant(left), constant(right)) {
            (Some(l), Some(r), _, _) => (l, Operand::Identity(r)),
            (Some(l), None, _, Some(c)) => (l, Operand::Constant(c)),
            (None, Some(r), Some(c), _) => (r, Operand::Constant(c)),
            _ => return Ok(result),
        };
        self.charge(self.proof.definitions.len() + 1)?;
        // The same product again (factors in either order).
        let again = self.proof.definitions.iter().find(|d| {
            d.multiply
                && match (d.right, second) {
                    (Operand::Identity(a), Operand::Identity(b)) => {
                        (d.left == first && a == b) || (d.left == b && a == first)
                    }
                    (Operand::Constant(a), Operand::Constant(b)) => d.left == first && a == b,
                    _ => false,
                }
        });
        if let Some(found) = again.copied() {
            let mut value = result;
            value.identity = Some(found.result);
            return self.scalar_facts(value);
        }
        let result = self.ensure_identity(result)?;
        let identity = result.identity.ok_or(SourceCheckError::Type)?;
        self.proof.definitions.push(Definition {
            result: identity,
            left: first,
            right: second,
            subtract: false,
            multiply: true,
            reflect: false,
        });
        Ok(result)
    }
}
