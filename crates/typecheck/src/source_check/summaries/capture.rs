//! Capture only the paths retained by the consumed body's original parameter roots.
use super::*;

impl Inputs {
    pub(super) fn capture(checker: &mut Checker<'_>) -> Result<Self> {
        checker.charge(checker.proof.names.len() * 3)?;
        let values: Vec<_> = checker
            .proof
            .names
            .iter()
            .map(|(_, value)| *value)
            .collect();
        let roots = (0..values.len()).collect();
        let mut inputs = Self {
            values,
            nodes: Vec::new(),
            roots,
        };
        while inputs.nodes.len() < inputs.values.len() {
            checker.charge(1)?;
            let index = inputs.nodes.len();
            let value = checker.owner_view(inputs.values[index])?;
            inputs.values[index] = value;
            let mut children = Vec::new();
            let mut slots = Vec::new();
            // A borrowed input is never consumed or returned, and a body may have changed what
            // it holds, so its structure is not part of what a caller must match.
            if let Some(owner) = value.owner
                && !checker.proof.owners[owner].passing.borrows()
            {
                let count = checker.proof.owners[owner].children.len();
                checker.charge(count * 2 + checker.proof.owners[owner].slots.len())?;
                slots = checker.proof.owners[owner].slots.clone();
                for child in &checker.proof.owners[owner].children {
                    children.push(inputs.values.len());
                    inputs.values.push(*child);
                }
            }
            // A number's input type is the range it was declared with, not whatever an
            // assignment later narrowed it to.
            let ty = match (value.ty, value.integer_domain) {
                (SourceType::Integer { .. }, Some((lower, upper))) => {
                    SourceType::Integer { lower, upper }
                }
                (ty, _) => ty,
            };
            inputs.nodes.push(Input {
                ty,
                children,
                slots,
            });
        }
        assert_eq!(inputs.nodes.len(), inputs.values.len());
        Ok(inputs)
    }

    /// Aliases always use the first captured identity, never an anonymous value.
    /// Callers prepay the lookup at their original capture charge sites.
    fn origin(&self, identity: Option<u32>) -> Option<usize> {
        self.values
            .iter()
            .position(|other| identity.is_some() && identity == other.identity)
    }

    pub(super) fn finish(
        &self,
        checker: &mut Checker<'_>,
        returned: Option<Value>,
    ) -> Result<Summary> {
        checker.charge(self.nodes.len() + self.roots.len() + 1)?;
        for node in &self.nodes {
            checker.charge(node.children.len() + node.slots.len())?;
        }
        let outputs = OutputCapture::capture(checker, self, returned)?;
        let dispositions = self.dispositions(checker, &outputs)?;
        checker.charge_product(checker.proof.integer_guards.len(), self.values.len() + 1)?;
        let integer_guards = checker
            .proof
            .integer_guards
            .iter()
            .filter_map(|(identity, number)| {
                self.origin(Some(*identity)).map(|input| (input, *number))
            })
            .collect();
        let mut input_ranges = Vec::new();
        for &root in &self.roots {
            let value = self.values[root];
            let (Some(identity), SourceType::Integer { lower, upper }) = (value.identity, self.nodes[root].ty) else {
                continue;
            };
            let Some(&(_, low, high)) = checker.proof.scalars.iter().find(|(id, _, _)| *id == identity) else {
                continue;
            };
            if low > lower || high < upper {
                input_ranges.push((root, low.max(lower), high.min(upper)));
            }
        }
        let borrowed = self
            .roots
            .iter()
            .map(|root| {
                self.values[*root]
                    .owner
                    .is_some_and(|owner| checker.proof.owners[owner].passing.borrows())
            })
            .collect();
        Ok(Summary {
            inputs: self.nodes.clone(),
            roots: self.roots.clone(),
            borrowed,
            dispositions,
            outputs,
            integer_guards,
            input_ranges,
        })
    }

    fn dispositions(
        &self,
        checker: &mut Checker<'_>,
        outputs: &[Output],
    ) -> Result<Vec<Disposition>> {
        checker.charge_product(self.nodes.len(), 2)?;
        checker.charge(outputs.len())?;
        let mut returned = vec![false; self.nodes.len()];
        let mut pending: Vec<_> = outputs.iter().filter_map(|output| output.input).collect();
        while let Some(index) = pending.pop() {
            checker.charge(1)?;
            if returned[index] {
                continue;
            }
            returned[index] = true;
            checker.charge(self.nodes[index].children.len())?;
            pending.extend_from_slice(&self.nodes[index].children);
        }
        let mut dispositions = Vec::new();
        for (index, input) in self.nodes.iter().enumerate() {
            let disposition = match input.ty {
                SourceType::Integer { .. }
                | SourceType::Unit
                | SourceType::Text
                | SourceType::Pointer { .. } => Disposition::Scalar,
                // Unrestricted values carry no obligation, so nothing needs to consume them,
                // and a view owns nothing it holds.
                SourceType::Aggregate { .. } | SourceType::Sum { .. }
                    if self.values[index].owner.is_some_and(|owner| {
                        checker.proof.owners[owner].unrestricted
                            || checker.proof.owners[owner].view
                    }) =>
                {
                    Disposition::Scalar
                }
                // A domain has no children to unpack; whether it was consumed was already
                // enforced when its owner was checked at the end of the body.
                SourceType::Aggregate { id }
                    if checker.module.layouts[checker.module.instances[id as usize].schema()]
                        .schema
                        .kind
                        == AggregateKind::Domain
                        && !returned[index] =>
                {
                    Disposition::Scalar
                }
                // A borrow is never consumed or returned: the owner keeps it untouched.
                _ if self.values[index]
                    .owner
                    .is_some_and(|owner| checker.proof.owners[owner].passing.borrows()) =>
                {
                    Disposition::Scalar
                }
                _ if returned[index] => Disposition::Returned,
                SourceType::Aggregate { .. }
                    if self.values[index]
                        .owner
                        .is_some_and(|owner| checker.proof.owners[owner].materialized) =>
                {
                    Disposition::Unpacked
                }
                // A body-less function (an instruction, a trait's signature) hands a record to
                // what implements it, which ends it.
                SourceType::Aggregate { .. }
                    if self.values[index]
                        .owner
                        .is_some_and(|owner| checker.proof.owners[owner].drained) =>
                {
                    Disposition::Ended
                }
                SourceType::Aggregate { .. } => return Err(SourceCheckError::Unconsumed),
                SourceType::Array { .. } | SourceType::Sequence { .. }
                    if self.values[index]
                        .owner
                        .is_some_and(|owner| checker.proof.owners[owner].drained) =>
                {
                    Disposition::Unpacked
                }
                SourceType::Array { .. } | SourceType::Sequence { .. } => {
                    return Err(SourceCheckError::Unconsumed);
                }
                // Only a `#instruction` drains a generic input: it hands it to the machine.
                SourceType::Generic { .. }
                    if self.values[index]
                        .owner
                        .is_some_and(|owner| checker.proof.owners[owner].drained) =>
                {
                    Disposition::Ended
                }
                SourceType::Generic { .. } => return Err(SourceCheckError::Unconsumed),
                SourceType::Sum { .. } => return Err(SourceCheckError::Type),
            };
            dispositions.push(disposition);
        }
        Ok(dispositions)
    }
}

/// The output prefix is complete; the remaining values are its BFS frontier.
/// Forwarded input subtrees never enter that frontier, even if materialized.
struct OutputCapture {
    values: Vec<Value>,
    nodes: Vec<Output>,
}

impl OutputCapture {
    fn capture(
        checker: &mut Checker<'_>,
        inputs: &Inputs,
        returned: Option<Value>,
    ) -> Result<Vec<Output>> {
        let mut capture = Self {
            values: returned.into_iter().collect(),
            nodes: Vec::new(),
        };
        while capture.nodes.len() < capture.values.len() {
            checker.charge(inputs.values.len() + 1)?;
            let value = checker.owner_view(capture.values[capture.nodes.len()])?;
            let value = checker.scalar_facts(value)?;
            let input = inputs.origin(value.identity);
            capture.append(checker, value, input)?;
        }
        assert_eq!(capture.nodes.len(), capture.values.len());
        Ok(capture.nodes)
    }

    fn append(
        &mut self,
        checker: &mut Checker<'_>,
        value: Value,
        input: Option<usize>,
    ) -> Result<()> {
        let mut children = Vec::new();
        let mut slots = Vec::new();
        let made_by_machine = input.is_none()
            && matches!(value.ty, SourceType::Generic { .. })
            && value
                .owner
                .is_some_and(|owner| checker.proof.owners[owner].drained);
        if input.is_none() {
            // Nothing can invent a value of an unknown type, except the machine: a
            // `#instruction` result is marked as coming from it.
            let produced = value
                .owner
                .is_some_and(|owner| checker.proof.owners[owner].drained);
            if matches!(value.ty, SourceType::Generic { .. }) && !produced {
                return Err(SourceCheckError::Unconsumed);
            }
            if let Some(owner) = value.owner {
                checker.charge(
                    checker.proof.owners[owner].children.len() * 2
                        + checker.proof.owners[owner].slots.len(),
                )?;
                slots = checker.proof.owners[owner].slots.clone();
                for child in &checker.proof.owners[owner].children {
                    children.push(self.values.len());
                    self.values.push(*child);
                }
            }
        }
        self.nodes.push(Output {
            produced: made_by_machine,
            ty: value.ty,
            lower: value.lower,
            upper: value.upper,
            input,
            children,
            slots,
            sum: value.sum,
            materialized: value
                .owner
                .is_some_and(|owner| checker.proof.owners[owner].materialized),
            structural: value.structural,
            integer_domain: value.integer_domain,
        });
        Ok(())
    }
}
