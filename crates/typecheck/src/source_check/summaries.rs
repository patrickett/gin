//! Flat body-derived origins. Input nodes denote existing values, never fresh owners.
use super::*;

mod capture;
#[cfg(test)]
mod inventory_tests;
mod replay;

use replay::{InputBindings, OutputAttachment};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Input {
    pub ty: SourceType,
    /// Only materialized paths occur here. An empty list can denote an opaque
    /// owned subtree; its output origin forwards that entire original subtree.
    pub children: Vec<usize>,
    /// Array slot numbers parallel to children; nominal records are dense.
    pub slots: Vec<u32>,
}

/// Original roots and their captured BFS nodes share one private value inventory.
pub(super) struct Inputs {
    values: Vec<Value>,
    pub nodes: Vec<Input>,
    roots: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Output {
    pub ty: SourceType,
    pub lower: I256,
    pub upper: I256,
    pub input: Option<usize>,
    pub children: Vec<usize>,
    pub slots: Vec<u32>,
    pub sum: Option<u32>,
    /// Empty children can mean an unrevealed fresh representation, not an empty record.
    pub materialized: bool,
    pub structural: Option<SourceType>,
    /// Coverage view survives calls independently of narrowed actual bounds.
    pub integer_domain: Option<(I256, I256)>,
    /// A fresh value of a type parameter that only the machine can make (a `#instruction` or
    /// `#load` result), passed on through whoever returns it.
    pub produced: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Summary {
    pub inputs: Vec<Input>,
    pub roots: Vec<usize>,
    /// Parallel to `roots`: the parameter only borrowed its argument, which the caller
    /// keeps and still has to end.
    pub borrowed: Vec<bool>,
    pub dispositions: Vec<Disposition>,
    pub outputs: Vec<Output>,
    /// Equality guards reference flat input origins, never a parameter's type shape.
    pub integer_guards: Vec<(usize, I256)>,
    /// The range this world knows of a number it was handed, where a guard it took made it
    /// narrower than the declared range (`value < 0` on the path that returns `0 - value`): an
    /// argument outside it cannot take this world.
    pub input_ranges: Vec<(usize, I256, I256)>,
}

/// Unpacked means the verified body structurally consumed a known aggregate. Ended means it
/// ended an opaque generic value by handing it to a trait method that takes it by value (or to a
/// machine instruction), the only way a generic value may end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Disposition {
    Scalar,
    Returned,
    Unpacked,
    Ended,
}

impl Summary {
    /// The ranges of an equal summary from another world join this one's: an argument is
    /// feasible when either world could take it, so each range widens to cover both, and a
    /// number one world left unconstrained is unconstrained.
    pub(super) fn absorb_ranges(&mut self, other: &Self) {
        let mut joined = Vec::new();
        for &(input, low, high) in &self.input_ranges {
            if let Some(&(_, other_low, other_high)) =
                other.input_ranges.iter().find(|(found, _, _)| *found == input)
            {
                joined.push((input, low.min(other_low), high.max(other_high)));
            }
        }
        self.input_ranges = joined;
    }

    pub(super) fn same(&self, other: &Self, remaining: &mut u32) -> Result<bool> {
        prepay(remaining, 1 + self.integer_guards.len())?;
        if self.integer_guards != other.integer_guards
            || self.inputs.len() != other.inputs.len()
            || self.outputs.len() != other.outputs.len()
        {
            return Ok(false);
        }
        for (left, right) in self.inputs.iter().zip(&other.inputs) {
            prepay(remaining, 1)?;
            if left.ty != right.ty {
                return Ok(false);
            }
            prepay(remaining, left.children.len() + left.slots.len())?;
            if left.children != right.children || left.slots != right.slots {
                return Ok(false);
            }
        }
        prepay(remaining, self.roots.len() + self.dispositions.len())?;
        if self.roots != other.roots
            || self.borrowed != other.borrowed
            || self.dispositions != other.dispositions
        {
            return Ok(false);
        }
        for (left, right) in self.outputs.iter().zip(&other.outputs) {
            prepay(remaining, left.children.len() + left.slots.len() + 1)?;
            if left != right {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl<'a> Checker<'a> {
    pub(super) fn summary_inputs(&mut self) -> Result<Inputs> {
        Inputs::capture(self)
    }

    pub(super) fn summarize(
        &mut self,
        inputs: &Inputs,
        returned: Option<Value>,
    ) -> Result<Summary> {
        inputs.finish(self, returned)
    }

    pub(super) fn apply_summary(
        &mut self,
        summary: &Summary,
        arguments: &[Value],
        types: &[SourceType],
    ) -> Result<Value> {
        let inputs = InputBindings::apply(self, summary, arguments)?;
        OutputAttachment::replay(self, inputs, types)
    }

    pub(super) fn apply_outcomes(
        &mut self,
        summaries: &[Summary],
        arguments: &[Value],
        types: &[SourceType],
    ) -> Result<Value> {
        self.charge(summaries.len())?;
        let mut feasible = Vec::new();
        for (index, summary) in summaries.iter().enumerate() {
            if InputBindings::feasible(self, summary, arguments, types)? {
                feasible.push(index);
            }
        }
        let selected = self.choose_outcome(feasible.len())?;
        self.apply_summary(&summaries[feasible[selected]], arguments, types)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_preserve_existing_child_and_scalar_identities() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        let generic = SourceType::Generic { index: 0 };
        let payload = checker.fresh_value(generic).unwrap();
        let scalar = checker
            .fresh_value(SourceType::Integer {
                lower: I256::ZERO,
                upper: I256::from(16),
            })
            .unwrap();
        checker.proof.names.push(("payload", payload));
        checker.proof.names.push(("scalar", scalar));
        let inputs = checker.summary_inputs().unwrap();
        // An outer constructor is fresh, but its children are the input values.
        let outer = checker
            .fresh_value(SourceType::Aggregate { id: 0 })
            .unwrap();
        checker.proof.owners[outer.owner.unwrap()].children = vec![payload, scalar];
        let summary = checker.summarize(&inputs, Some(outer)).unwrap();
        assert!(summary.outputs[0].input.is_none());
        assert_eq!(summary.outputs[1].input, Some(0));
        assert_eq!(summary.outputs[2].input, Some(1));
        // Applying the child-only summary must return the original child, not an
        // opaque value synthesized from its result type.
        let child_summary = checker.summarize(&inputs, Some(payload)).unwrap();
        let returned = checker
            .apply_summary(&child_summary, &[payload, scalar], &[generic])
            .unwrap();
        assert_eq!(returned.owner, payload.owner);
        assert_eq!(returned.identity, payload.identity);
        checker.proof.names.clear();
        checker.proof.names.push(("scalar", scalar));
        let scalar_inputs = checker.summary_inputs().unwrap();
        let scalar_summary = checker.summarize(&scalar_inputs, Some(scalar)).unwrap();
        let returned = checker
            .apply_summary(&scalar_summary, &[scalar], &[])
            .unwrap();
        assert_eq!(returned.identity, scalar.identity);
    }
}
