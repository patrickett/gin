//! Body-free, positional scalar contracts shared by local and imported schemas.
use super::*;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub(super) struct Origin {
    pub module: Arc<()>,
    pub declaration: usize,
}

impl PartialEq for Origin {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.module, &other.module) && self.declaration == other.declaration
    }
}

#[derive(Debug, Clone)]
pub(super) struct Layout {
    pub origin: Origin,
    pub schema: CheckedSourceAggregate,
    pub constraints: Vec<Constraint>,
    /// The stable name of a layout the checker made (`f#result`, `f#labeled#0`), unique in
    /// its module; `None` for a declared type or variant, whose `schema.name` is that name.
    pub key: Option<String>,
}

impl Layout {
    pub(super) fn prepay(&self, remaining: &mut u32) -> Result<()> {
        prepay(
            remaining,
            self.schema.name.len()
                + self.schema.parameters.len()
                + self.schema.fields.len()
                + self.schema.variants.len()
                + self.constraints.len()
                + 1,
        )?;
        for parameter in &self.schema.parameters {
            prepay(remaining, parameter.len())?;
        }
        for field in &self.schema.fields {
            prepay(remaining, field.name.len() + 1)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) enum Term {
    Slot(usize),
    Constant(I256),
    /// A number type parameter of the enclosing type, by generic index.
    Number(u32),
    /// The scalar with this identity: a caller's own abstract number, carried into a callee's
    /// claim at a call (see `numbers.rs`).
    Identity(u32),
    /// A field of a parameter: `self.len`. The field is a position in its record.
    Path { slot: usize, field: u32, deeper: Vec<u32> },
    /// `left + right`, `left - right` or `left * right`, worked out over intervals.
    Arithmetic(CheckedArithmetic, Box<Term>, Box<Term>),
    /// `#size(T)` or `#alignment(T)` of a type that is still a type parameter here.
    Layout { alignment: bool, ty: SourceType },
}

#[derive(Debug, Clone)]
pub(super) struct Constraint {
    pub left: usize,
    /// The claim is about this field of the parameter (`buffer.length < cap`), not the
    /// parameter itself.
    pub left_field: Option<u32>,
    /// The claim is about this arithmetic over parameters (`count * #size(element)`), not a
    /// single one: it is checked at each use and assumes nothing.
    pub left_term: Option<Term>,
    /// `left % modulus = 0`: the claim is that the number is a multiple of this.
    pub modulus: Option<I256>,
    pub operator: BinaryOperator,
    pub right: Term,
}

impl<'a> Checker<'a> {
    pub(super) fn prepare_layouts(&mut self) -> diagnostics::DiagnosticResult<()> {
        self.charge(self.module.schemas.len() + 1)?;
        let module = self.module.origin.clone();
        for index in 0..self.module.schemas.len() {
            let declaration = self.module.schemas[index];
            let context = declaration
                .parameters
                .as_ref()
                .map_or(declaration.name.span_id, |parameters| parameters.span_id);
            let scope = self
                .schema_scope(index)
                .map_err(|error| self.failure(error, context))?;
            let name = &self.module.schemas[index].name.value;
            self.charge(name.len() + 1)?;
            let mut parameters = Vec::new();
            for (name, _) in scope {
                self.charge(name.len() + 1)?;
                parameters.push(name.to_owned());
            }
            let name = self.module.schemas[index].name.value.clone();
            self.module.type_names.push((name.clone(), index));
            self.module.layouts.push(Layout {
                origin: Origin {
                    module: module.clone(),
                    declaration: index,
                },
                schema: CheckedSourceAggregate {
                    name,
                    parameters,
                    fields: Vec::new(),
                    variants: Vec::new(),
                    capability: self.module.schema_capability[index],
                    kind: AggregateKind::of(declaration),
                    wrapping: self.module.schema_wrap[index],
                    defaults: Vec::new(),
                    bounds: Vec::new(),
                    provides: Vec::new(),
                    borrowing: Borrowing::None,
                    numbers: Vec::new(),
                    domain_claims: Vec::new(),
                    discriminant: None,
                },
                constraints: Vec::new(),
                key: None,
            });
        }
        Ok(())
    }

    pub(super) fn compile_layouts(&mut self, schemas: schema_validation::ValidatedSourceSchemas) {
        // Nominal origins and parameter shells already exist before imports.
        // Installation consumes only fully validated, source-free local packets.
        schemas.install(&mut self.module.layouts[..self.module.schemas.len()]);
    }

    pub(super) fn compile_constraints(
        &mut self,
        annotation: &'a Spanned<Annotation>,
        left: usize,
        names: &[&str],
        output: &mut Vec<Constraint>,
    ) -> Result<()> {
        // `n in 0 to 9 and < 5` and `n Length and n <= capacity` refine the same way.
        let (parent, refinements) = match &annotation.value {
            Annotation::Domain { domain, .. } => (domain.span_id, &domain.refinements),
            Annotation::Refined { refinements, .. } => (annotation.span_id, refinements),
            _ => return Ok(()),
        };
        self.charge(refinements.len())?;
        for refinement in refinements {
            self.child(parent, refinement.span_id)?;
            self.child(refinement.span_id, refinement.and_keyword)?;
            self.child(refinement.span_id, refinement.predicate.span_id)?;
            self.token(refinement.and_keyword, "and")?;
            let right = match &refinement.predicate.value {
                RefinementPredicate::ImplicitComparison { operator, right } => {
                    self.comparison(operator)?;
                    self.child(refinement.predicate.span_id, operator.span_id)?;
                    self.child(refinement.predicate.span_id, right.span_id)?;
                    let right = self.contract_term(right, names)?;
                    output.push(Constraint {
                        left,
                        left_field: None, left_term: None, modulus: None,
                        operator: operator.value,
                        right,
                    });
                    None
                }
                RefinementPredicate::ImplicitDomain(domain) => {
                    // `and in L to U` is `>= L` and `<= U` at the contract level.
                    let DomainBase::Range {
                        lower,
                        upper,
                        to_keyword,
                    } = &domain.base.value
                    else {
                        return Err(SourceCheckError::Unsupported);
                    };
                    if let Some(keyword) = domain.in_keyword {
                        self.keyword(keyword, "in")?;
                    }
                    self.child(refinement.predicate.span_id, domain.span_id)?;
                    self.child(domain.span_id, domain.base.span_id)?;
                    self.child(domain.base.span_id, lower.span_id)?;
                    self.child(domain.base.span_id, upper.span_id)?;
                    self.token(*to_keyword, "to")?;
                    Some((lower, upper))
                }
                RefinementPredicate::Expression(relation) => {
                    self.relation_constraint(relation, names, output)?;
                    None
                }
            };
            if let Some((lower, upper)) = right {
                for (operator, expression) in [
                    (BinaryOperator::GreaterOrEqual, lower),
                    (BinaryOperator::LessOrEqual, upper),
                ] {
                    let right = self.contract_term(expression, names)?;
                    output.push(Constraint {
                        left,
                        left_field: None, left_term: None, modulus: None,
                        operator,
                        right,
                    });
                }
            }
        }
        Ok(())
    }

    pub(super) fn contract_term(&mut self, root: &'a Expression, names: &[&str]) -> Result<Term> {
        let mut expression = root;
        loop {
            self.charge(1)?;
            self.span(expression.span_id)?;
            if let Expr::Group(child) = &expression.value {
                self.group_syntax(expression, child)?;
                expression = child;
                continue;
            }
            if let Expr::Name(name) = &expression.value {
                self.token(expression.span_id, name)?;
                self.charge_product(names.len(), name.len() + 1)?;
                let Some(slot) = names.iter().position(|other| *other == name) else {
                    // Not a parameter: a number parameter of the enclosing type.
                    return match self.number_term(name) {
                        // Neither: a module constant is a number known now.
                        Err(SourceCheckError::UnknownName) => self
                            .constant_value(name)?
                            .map(Term::Constant)
                            .ok_or(SourceCheckError::UnknownName),
                        other => other,
                    };
                };
                return Ok(Term::Slot(slot));
            }
            if let Expr::Member { base, name } = &expression.value {
                return self.path_term(expression, base, name, names);
            }
            // The pure expression evaluator rejects calls. Only literal constants are
            // admitted here; names and their negation cannot become captured metadata.
            if let Some(term) = self.arithmetic_term(expression, names)? {
                // Literals and constants only: already a number.
                if let Term::Arithmetic(..) = &term
                    && let Ok(number) = self.constant_expression(expression)
                {
                    return Ok(Term::Constant(number));
                }
                return Ok(term);
            }
            // Literals and module constants combined (`capacity - 4`) are a number known now.
            match self.constant_expression(expression) {
                Ok(number) => return Ok(Term::Constant(number)),
                Err(SourceCheckError::Budget) => return Err(SourceCheckError::Budget),
                Err(_) => {}
            }
            let value = self.expression(expression)?;
            self.scalar(value)?;
            if value.identity.is_some() || value.lower != value.upper {
                return Err(SourceCheckError::Unsupported);
            }
            return Ok(Term::Constant(value.lower));
        }
    }

    pub(super) fn constraints(
        &mut self,
        constraints: &[Constraint],
        values: &mut [Value],
        assume: bool,
    ) -> Result<()> {
        self.charge(constraints.len())?;
        for constraint in constraints {
            if let Some(term) = &constraint.left_term {
                // Nothing can be assumed of a product by itself; at a use it must be shown.
                if !assume {
                    // The sums on either side are related to their operands, so a claim stated
                    // as `(a + b) + c` and one reached as `a + (b + c)` meet in the linear facts.
                    self.proof.relate_always = true;
                    let left = self.term_value(term, values);
                    let right = self.term_value(&constraint.right, values);
                    self.proof.relate_always = false;
                    self.compare(constraint.operator, left?, right?)?;
                } else {
                    self.assume_term_claim(term, constraint, values)?;
                }
                continue;
            }
            let mut left = values[constraint.left];
            if let Some(field) = constraint.left_field {
                left = self.field_child(left, field)?;
            }
            if let Some(modulus) = constraint.modulus {
                let number = self.open_number(left)?;
                if assume {
                    self.note_multiple(number, modulus)?;
                } else if !self.proves_multiple(number, modulus)? {
                    let detail = format!("this number is not shown to be a multiple of {modulus}");
                    return Err(self.explain(SourceCheckError::Unproved, detail));
                }
                continue;
            }
            // Assuming `n = first + count`: the right side is related to its operands, so the
            // claim becomes a fact about all of them.
            self.proof.relate_always = assume && matches!(constraint.right, Term::Arithmetic(..));
            let right = self.term_value(&constraint.right, values);
            self.proof.relate_always = false;
            let right = right?;
            // A domain compares as the number it holds.
            let numeric = self.domain_number(left)?;
            let right = self.domain_number(right)?;
            if assume {
                if right.identity.is_some() {
                    self.assume(constraint.operator, numeric, right)?;
                } else if (numeric.ty != left.ty || constraint.left_field.is_some())
                    && numeric.identity.is_some()
                {
                    // The number a domain holds, or a field of a parameter, is not a value of
                    // its own to replace, so the claim becomes a fact about it.
                    let right = self.ensure_identity(right)?;
                    self.assume(constraint.operator, numeric, right)?;
                } else {
                    let narrowed = self.assume_constant(constraint.operator, numeric, right)?;
                    if numeric.ty == left.ty && constraint.left_field.is_none() {
                        values[constraint.left] = narrowed;
                    }
                }
            } else {
                self.compare(constraint.operator, numeric, right)?;
            }
        }
        Ok(())
    }

    fn assume_constant(
        &mut self,
        operator: BinaryOperator,
        mut left: Value,
        right: Value,
    ) -> Result<Value> {
        self.scalar(left)?;
        self.scalar(right)?;
        match operator {
            BinaryOperator::Equal => {
                left.lower = left.lower.max(right.lower);
                left.upper = left.upper.min(right.upper);
            }
            BinaryOperator::GreaterOrEqual => left.lower = left.lower.max(right.lower),
            BinaryOperator::LessOrEqual => left.upper = left.upper.min(right.upper),
            BinaryOperator::Greater => {
                left.lower = left.lower.max(
                    right
                        .lower
                        .checked_add(I256::ONE)
                        .ok_or(SourceCheckError::Type)?,
                );
            }
            BinaryOperator::Less => {
                left.upper = left.upper.min(
                    right
                        .upper
                        .checked_sub(I256::ONE)
                        .ok_or(SourceCheckError::Type)?,
                );
            }
            _ => return Err(SourceCheckError::Unsupported),
        }
        if left.lower > left.upper {
            return Err(SourceCheckError::Type);
        }
        left.ty = SourceType::Integer {
            lower: left.lower,
            upper: left.upper,
        };
        self.narrow_scalar(left)?;
        Ok(left)
    }

    pub(super) fn projection(&mut self, base: &'a Expression, name: &'a Name) -> Result<Value> {
        self.name(name)?;
        let mut expression = base;
        loop {
            self.charge(1)?;
            let Expr::Group(child) = &expression.value else {
                break;
            };
            self.group_syntax(expression, child)?;
            expression = child;
        }
        let Expr::Name(base_name) = &expression.value else {
            return Err(SourceCheckError::Unsupported);
        };
        self.token(expression.span_id, base_name)?;
        self.charge_product(self.proof.names.len(), base_name.len() + 1)?;
        let value = self
            .proof
            .names
            .iter()
            .find(|(other, _)| *other == base_name)
            .map(|(_, value)| *value)
            .ok_or(SourceCheckError::UnknownName)?;
        self.projection_value(value, name)
    }

    /// A field of a computed record: not supported, but a field that may never be named
    /// (private to another module, or a domain's hidden number) is reported as private.
    pub(super) fn computed_projection(&mut self, value: Value, name: &Name) -> Result<Value> {
        if let SourceType::Aggregate { id } = value.ty {
            let schema = self.module.instances[id as usize].schema();
            self.charge_product(
                self.module.layouts[schema].schema.fields.len(),
                name.value.len() + 1,
            )?;
            let layout = &self.module.layouts[schema].schema;
            let hidden = layout.kind == AggregateKind::Domain
                || (schema >= self.module.schemas.len()
                    && layout
                        .fields
                        .iter()
                        .any(|field| field.name == name.value && field.private));
            if hidden {
                return Err(SourceCheckError::Privacy);
            }
            // `call(...).field`: the call's result is evaluated and the field taken out of it.
            // The other fields go with the temporary, so none may still owe an ending.
            let layout = &self.module.layouts[schema].schema;
            let position = layout.fields.iter().position(|field| field.name == name.value);
            let Some(position) = position else {
                return Err(SourceCheckError::UnknownName);
            };
            let projected = self.projection_value(value, name)?;
            let owner = value.owner.ok_or(SourceCheckError::Type)?;
            self.materialize_children(value)?;
            self.charge(self.proof.owners[owner].children.len())?;
            let others_owe = self.proof.owners[owner]
                .children
                .iter()
                .enumerate()
                .any(|(index, child)| index != position && self.holds_linear(*child));
            if others_owe {
                let detail = "the other fields of this result still owe an ending: bind the result and take its fields apart".to_owned();
                return Err(self.explain(SourceCheckError::Unconsumed, detail));
            }
            self.release(value)?;
            return Ok(projected);
        }
        Err(SourceCheckError::Unsupported)
    }

    /// The field `name` of the record `value`, as the record's own child. A field of an
    /// imported schema marked private, and the hidden number inside a domain, can never be
    /// named. Taking the child moves nothing; what a caller may then do with it, copy or
    /// only lend, depends on whether its type is unrestricted.
    pub(super) fn projection_value(&mut self, value: Value, name: &Name) -> Result<Value> {
        let value = self.owner_view(value)?;
        let SourceType::Aggregate { id } = value.ty else {
            return Err(SourceCheckError::Unsupported);
        };
        let schema = self.module.instances[id as usize].schema();
        self.charge_product(
            self.module.layouts[schema].schema.fields.len(),
            name.value.len() + 1,
        )?;
        let layout = &self.module.layouts[schema].schema;
        let direct = layout.fields.iter().position(|field| field.name == name.value);
        // Not a field: an inline accessor of the type reads the field it returns, whether or
        // not that field is private (the accessor is the type's own public reading of it).
        let accessed = match direct {
            Some(_) => None,
            None => {
                let owner = layout.name.clone();
                self.charge(self.accessors.len())?;
                let field = self
                    .accessors
                    .iter()
                    .find(|(type_name, method, _)| *type_name == owner && *method == name.value)
                    .map(|(_, _, field)| field.clone())
                    .ok_or(SourceCheckError::UnknownName)?;
                // An accessor that reads deeper (`return self.storage.length`): its first field,
                // then each further member (a field or an accessor) of what that holds.
                if let Some((head, rest)) = field.split_once('.') {
                    let head = Name::new(head.to_owned(), name.span_id);
                    let mut current = self.projection_value_unchecked(value, &head)?;
                    for segment in rest.split('.') {
                        current = self.projection_value(current, &Name::new(segment.to_owned(), name.span_id))?;
                    }
                    return Ok(current);
                }
                let layout = &self.module.layouts[schema].schema;
                Some(
                    layout
                        .fields
                        .iter()
                        .position(|candidate| candidate.name == field)
                        .ok_or(SourceCheckError::UnknownName)?,
                )
            }
        };
        let layout = &self.module.layouts[schema].schema;
        let index = direct.or(accessed).ok_or(SourceCheckError::UnknownName)?;
        if layout.kind == AggregateKind::Domain
            || (accessed.is_none() && schema >= self.module.schemas.len() && layout.fields[index].private)
        {
            return Err(SourceCheckError::Privacy);
        }
        self.materialize_children(value)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        Ok(self.proof.owners[owner].children[index])
    }
}

impl<'a> Checker<'a> {
    /// Assume `term <operator> right` where `term` is arithmetic over parameters: when the
    /// sum or difference has an identity (it is related to the numbers it is made of), the
    /// claim becomes a fact about it, and so about them (`linear.rs`). A product or a term
    /// with no identity still assumes nothing.
    fn assume_term_claim(
        &mut self,
        term: &Term,
        constraint: &Constraint,
        values: &mut [Value],
    ) -> Result<()> {
        self.proof.relate_always = true;
        let attempt = (|| -> Result<()> {
            let left = self.term_value(term, values)?;
            let right = self.term_value(&constraint.right, values)?;
            // A claim over a field of a domain type speaks of the number the domain holds.
            let left = self.domain_number(left)?;
            let right = self.domain_number(right)?;
            if left.identity.is_none() {
                return Ok(());
            }
            // A bound by a constant is a fact about the number too: the constant is given an
            // identity so the relation is kept (`buffer.capacity >= 19` at a loop head), where
            // narrowing a copy of the value would be lost.
            let right = self.ensure_identity(right)?;
            self.assume(constraint.operator, left, right)
        })();
        self.proof.relate_always = false;
        match attempt {
            // A claim that cannot be read as a fact is simply not assumed, as before.
            Err(SourceCheckError::Budget) => Err(SourceCheckError::Budget),
            _ => Ok(()),
        }
    }
}

impl<'a> Checker<'a> {
    /// A field of `value` by name, read for the type's own accessor (no visibility check).
    fn projection_value_unchecked(&mut self, value: Value, name: &Name) -> Result<Value> {
        let value = self.owner_view(value)?;
        let SourceType::Aggregate { id } = value.ty else {
            return Err(SourceCheckError::Unsupported);
        };
        let index = self.field_index(id, &name.value)?;
        self.materialize_children(value)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        Ok(self.proof.owners[owner].children[index])
    }
}
