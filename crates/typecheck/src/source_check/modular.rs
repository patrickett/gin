//! Refinements of a domain declaration and the divisibility facts they need.
//!
//! `MappedAddress is in 0 to top and self % 4096 = 0` narrows a range by a claim. Every value
//! converted into the domain must be shown to satisfy it (an `Unproved` cast otherwise), and a
//! value read out of the domain carries it. Comparisons are settled by the value's interval; a
//! divisibility claim by a fact `(number, modulus)` that holds for a literal multiple, for a
//! parameter that claims it (`address and address % 4096 = 0`), or for a value of a domain
//! that declares it. A multiple of 8192 is a multiple of 4096.
use super::*;

impl<'a> Checker<'a> {
    /// The claims a domain declaration makes, with their constants worked out.
    pub(super) fn domain_claims(
        &mut self,
        declaration: &'a TypeDeclaration,
    ) -> Result<Vec<DomainClaim>> {
        let TypeBody::Domain(domain) = &declaration.body.value else {
            return Ok(Vec::new());
        };
        let mut claims = Vec::new();
        for clause in &domain.refinements {
            self.charge(4)?;
            match &clause.predicate.value {
                RefinementPredicate::ImplicitComparison { operator, right } => {
                    let number = self.constant_expression(right)?;
                    claims.push(DomainClaim::Compare(operator.value, number));
                }
                RefinementPredicate::ImplicitDomain(inner) => {
                    let DomainBase::Range { lower, upper, .. } = &inner.base.value else {
                        return Err(SourceCheckError::Unsupported);
                    };
                    claims.push(DomainClaim::Compare(
                        BinaryOperator::GreaterOrEqual,
                        self.constant_expression(lower)?,
                    ));
                    claims.push(DomainClaim::Compare(
                        BinaryOperator::LessOrEqual,
                        self.constant_expression(upper)?,
                    ));
                }
                RefinementPredicate::Expression(expression) => {
                    claims.push(self.domain_claim(expression)?);
                }
            }
        }
        Ok(claims)
    }

    fn domain_claim(&mut self, expression: &'a Expression) -> Result<DomainClaim> {
        let Expr::Binary { left, operator, right } = &expression.value else {
            return Err(SourceCheckError::Unsupported);
        };
        let is_self = |expression: &Expression| matches!(&expression.value, Expr::Name(name) if name == "self");
        if is_self(left)
            && matches!(
                operator.value,
                BinaryOperator::Equal
                    | BinaryOperator::Less
                    | BinaryOperator::LessOrEqual
                    | BinaryOperator::Greater
                    | BinaryOperator::GreaterOrEqual
            )
        {
            return Ok(DomainClaim::Compare(operator.value, self.constant_expression(right)?));
        }
        // `self % modulus = 0`
        if operator.value == BinaryOperator::Equal
            && let Expr::Binary { left: inner, operator: modulo, right: modulus } = &left.value
            && modulo.value == BinaryOperator::Remainder
            && is_self(inner)
            && self.constant_expression(right)? == I256::ZERO
        {
            let modulus = self.constant_expression(modulus)?;
            if modulus > I256::ZERO {
                return Ok(DomainClaim::Multiple(modulus));
            }
        }
        Err(SourceCheckError::Unsupported)
    }

    /// The claims of a domain type; none for any other type.
    pub(super) fn claims_of(&self, ty: SourceType) -> Vec<DomainClaim> {
        let SourceType::Aggregate { id } = ty else {
            return Vec::new();
        };
        let schema = self.module.instances[id as usize].schema();
        self.module.layouts[schema].schema.domain_claims.clone()
    }

    /// Whether `value` is known to be a multiple of `modulus`.
    pub(super) fn proves_multiple(&mut self, value: Value, modulus: I256) -> Result<bool> {
        self.charge(self.proof.multiples.len() + 1)?;
        if value.lower == value.upper {
            return Ok(value.lower.checked_rem(modulus) == Some(I256::ZERO));
        }
        let Some(identity) = value.identity else {
            return Ok(false);
        };
        Ok(self.proof.multiples.iter().any(|(known, held)| {
            *known == identity && held.checked_rem(modulus) == Some(I256::ZERO)
        }))
    }

    /// Record that `value` is a multiple of `modulus`.
    pub(super) fn note_multiple(&mut self, value: Value, modulus: I256) -> Result<()> {
        let value = self.ensure_identity(value)?;
        if let Some(identity) = value.identity
            && !self.proof.multiples.contains(&(identity, modulus))
        {
            self.charge(1)?;
            self.proof.multiples.push((identity, modulus));
        }
        Ok(())
    }

    /// A number converted into `expected` must satisfy what its declaration adds to the range.
    pub(super) fn prove_claims(&mut self, value: Value, expected: SourceType) -> Result<Value> {
        let claims = self.claims_of(expected);
        let mut value = value;
        for claim in claims {
            self.charge(2)?;
            let holds = match claim {
                DomainClaim::Compare(operator, number) => match operator {
                    BinaryOperator::Equal => value.lower == number && value.upper == number,
                    BinaryOperator::Less => value.upper < number,
                    BinaryOperator::LessOrEqual => value.upper <= number,
                    BinaryOperator::Greater => value.lower > number,
                    BinaryOperator::GreaterOrEqual => value.lower >= number,
                    _ => return Err(SourceCheckError::Unsupported),
                },
                DomainClaim::Multiple(modulus) => {
                    let holds = self.proves_multiple(value, modulus)?;
                    if holds {
                        value = self.ensure_identity(value)?;
                        self.note_multiple(value, modulus)?;
                    }
                    holds
                }
            };
            if !holds {
                let detail = match claim {
                    DomainClaim::Compare(operator, number) => {
                        format!("this number is not shown to be `self {} {number}`, which the domain declares", symbol(operator))
                    }
                    DomainClaim::Multiple(modulus) => {
                        format!("this number is not shown to be a multiple of {modulus}, which the domain declares")
                    }
                };
                return Err(self.explain(SourceCheckError::Unproved, detail));
            }
        }
        Ok(value)
    }

    /// A number read out of an open domain carries the domain's divisibility claims.
    pub(super) fn seed_claims(&mut self, number: Value, domain: SourceType) -> Result<()> {
        for claim in self.claims_of(domain) {
            if let DomainClaim::Multiple(modulus) = claim {
                self.note_multiple(number, modulus)?;
            }
        }
        Ok(())
    }
}

fn symbol(operator: BinaryOperator) -> &'static str {
    match operator {
        BinaryOperator::Equal => "=",
        BinaryOperator::Less => "<",
        BinaryOperator::LessOrEqual => "<=",
        BinaryOperator::Greater => ">",
        _ => ">=",
    }
}
