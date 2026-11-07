//! Ephemeral bindings of flat summary nodes to current original values.
use super::*;

/// No binding escapes feasibility. Selected application starts again at the
/// original arguments, releasing roots only on that path.
pub(super) struct InputBindings<'s> {
    summary: &'s Summary,
    values: Vec<Option<Value>>,
}

impl<'s> InputBindings<'s> {
    /// The caller reserves the binding inventory before either replay mode.
    fn new(summary: &'s Summary) -> Self {
        Self {
            summary,
            values: vec![None; summary.inputs.len()],
        }
    }

    fn original(&self, index: usize) -> Result<Value> {
        self.values[index].ok_or(SourceCheckError::Type)
    }

    pub(super) fn apply(
        checker: &mut Checker<'_>,
        summary: &'s Summary,
        arguments: &[Value],
    ) -> Result<Self> {
        assert_eq!(summary.roots.len(), arguments.len());
        assert_eq!(summary.inputs.len(), summary.dispositions.len());
        checker.charge(summary.inputs.len() + summary.outputs.len() + summary.roots.len())?;
        let mut bindings = Self::new(summary);
        for ((root, value), borrowed) in summary.roots.iter().zip(arguments).zip(&summary.borrowed)
        {
            bindings.values[*root] = Some(*value);
            // A lent argument stays with the caller, who still has to end it.
            if !*borrowed {
                checker.release(*value)?;
            }
        }
        for (index, node) in summary.inputs.iter().enumerate() {
            let value = checker.owner_view(bindings.original(index)?)?;
            bindings.values[index] = Some(value);
            if summary.dispositions[index] == Disposition::Ended
                && let Some(owner) = value.owner
            {
                // The callee ended this generic value; it is the caller's no more.
                checker.proof.owners[owner].drained = true;
                checker.proof.owners[owner].live = false;
            }
            if summary.dispositions[index] == Disposition::Unpacked {
                if matches!(
                    value.ty,
                    SourceType::Array { .. } | SourceType::Sequence { .. }
                ) {
                    let owner = value.owner.ok_or(SourceCheckError::Type)?;
                    checker.proof.owners[owner].drained = true;
                } else {
                    checker.materialize_children(value)?;
                }
            }
            if !node.children.is_empty() && !bindings.bind_children(checker, value, node)? {
                return Err(SourceCheckError::Type);
            }
        }
        checker.charge(summary.integer_guards.len())?;
        for (input, number) in &summary.integer_guards {
            checker.guard_integer(bindings.original(*input)?, *number)?;
        }
        Ok(bindings)
    }

    pub(super) fn feasible(
        checker: &mut Checker<'_>,
        summary: &'s Summary,
        arguments: &[Value],
        types: &[SourceType],
    ) -> Result<bool> {
        assert_eq!(summary.roots.len(), arguments.len());
        checker.charge(summary.inputs.len() + arguments.len())?;
        let mut bindings = Self::new(summary);
        for (root, argument) in summary.roots.iter().zip(arguments) {
            bindings.values[*root] = Some(*argument);
        }
        if !bindings.known_guards(checker)? {
            return Ok(false);
        }
        for (input, low, high) in &summary.input_ranges {
            let Some(argument) = bindings.values[*input] else { continue };
            if !matches!(argument.ty, SourceType::Integer { .. }) {
                continue;
            }
            let argument = checker.scalar_facts(argument)?;
            if argument.upper < *low || argument.lower > *high {
                return Ok(false);
            }
        }
        let guard_paths = bindings.guard_paths(checker)?;
        for guarded in [true, false] {
            for (index, needed) in guard_paths.iter().enumerate() {
                checker.charge(1)?;
                if *needed == guarded && !bindings.test_input(checker, index, types)? {
                    return Ok(false);
                }
            }
        }
        assert!(bindings.values.iter().all(Option::is_some));
        bindings.known_guards(checker)
    }

    /// Visit numeric guard ancestors before unrelated variant tests. Flat input
    /// nodes are topological, so one reverse pass finds every required ancestor.
    fn guard_paths(&self, checker: &mut Checker<'_>) -> Result<Vec<bool>> {
        let summary = self.summary;
        checker.charge(summary.inputs.len() + summary.integer_guards.len())?;
        let mut needed = vec![false; summary.inputs.len()];
        for (input, _) in &summary.integer_guards {
            needed[*input] = true;
        }
        for (index, input) in summary.inputs.iter().enumerate().rev() {
            checker.charge(input.children.len())?;
            for child in &input.children {
                assert!(*child > index);
                needed[index] |= needed[*child];
            }
        }
        Ok(needed)
    }

    fn test_input(
        &mut self,
        checker: &mut Checker<'_>,
        index: usize,
        types: &[SourceType],
    ) -> Result<bool> {
        let input = &self.summary.inputs[index];
        let mut value = checker.owner_view(self.original(index)?)?;
        let ty = checker.substitute(input.ty, types)?;
        // Generic nodes stay opaque. A sum stays opaque unless its verified body
        // inspected this place, making the active nominal variant an input guard.
        if !matches!(input.ty, SourceType::Generic { .. }) {
            if matches!(ty, SourceType::Aggregate { .. }) {
                value = checker.inspect_value(value)?;
            }
            match checker.compatible(ty, value.ty) {
                Ok(()) => {}
                Err(SourceCheckError::Type) => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        self.values[index] = Some(value);
        if !input.children.is_empty() {
            if !self.bind_children(checker, value, input)? {
                return Ok(false);
            }
            return self.known_guards(checker);
        }
        Ok(true)
    }

    /// Sparse positions are not child indices. Records instead bind their dense
    /// children in declaration order, after revealing exactly this one level.
    fn bind_children(
        &mut self,
        checker: &mut Checker<'_>,
        value: Value,
        input: &Input,
    ) -> Result<bool> {
        checker.charge(input.children.len() + input.slots.len())?;
        if matches!(
            value.ty,
            SourceType::Array { .. } | SourceType::Sequence { .. }
        ) {
            assert_eq!(input.children.len(), input.slots.len());
            for (slot, node) in input.slots.iter().zip(&input.children) {
                self.values[*node] = Some(checker.array_child(value, *slot)?);
            }
        } else {
            assert!(input.slots.is_empty());
            checker.materialize_children(value)?;
            let owner = value.owner.ok_or(SourceCheckError::Type)?;
            if checker.proof.owners[owner].children.len() != input.children.len() {
                return Ok(false);
            }
            for (node, child) in input
                .children
                .iter()
                .zip(&checker.proof.owners[owner].children)
            {
                self.values[*node] = Some(*child);
            }
        }
        Ok(true)
    }

    /// Prune impossible outcomes before they can inspect unrelated owned places.
    /// Recheck jointly whenever a record reveals more scalar origins, because
    /// distinct formal guard slots can alias one actual scalar.
    fn known_guards(&self, checker: &mut Checker<'_>) -> Result<bool> {
        checker.charge(self.summary.integer_guards.len())?;
        let mut guards = Vec::new();
        for (input, number) in &self.summary.integer_guards {
            if let Some(value) = self.values[*input] {
                guards.push((value, *number));
            }
        }
        if guards.is_empty() {
            return Ok(true);
        }
        checker.integer_guards_feasible(&guards)
    }
}

/// Values are constructed in flat output order, then fresh edges are attached.
/// Original subtrees are forwarded intact and never have their children replaced.
pub(super) struct OutputAttachment<'s> {
    nodes: &'s [Output],
    values: Vec<Value>,
}

impl<'s> OutputAttachment<'s> {
    pub(super) fn replay(
        checker: &mut Checker<'_>,
        inputs: InputBindings<'s>,
        types: &[SourceType],
    ) -> Result<Value> {
        let mut outputs = Self {
            nodes: &inputs.summary.outputs,
            values: Vec::new(),
        };
        for output in outputs.nodes {
            let value = if let Some(input) = output.input {
                inputs.original(input)?
            } else {
                let ty = checker.substitute(output.ty, types)?;
                // Nothing invents a value of an unknown type, except a `#load`, which reads it
                // from memory an `#unchecked(...)` cast vouched for (see `load_access`).
                if matches!(ty, SourceType::Generic { .. }) && !output.produced {
                    return Err(SourceCheckError::Type);
                }
                let mut value = checker.fresh_value(ty)?;
                // Still made by the machine for anyone this result is returned to.
                if output.produced
                    && let Some(owner) = value.owner
                {
                    checker.proof.owners[owner].drained = true;
                }
                if matches!(ty, SourceType::Integer { .. }) {
                    value.lower = output.lower;
                    value.upper = output.upper;
                    checker.narrow_scalar(value)?;
                }
                value
            };
            outputs
                .values
                .push(Self::view(checker, value, output, types)?);
        }
        outputs.attach(checker)?;
        Ok(outputs.values.first().copied().unwrap_or_else(Value::unit))
    }

    fn view(
        checker: &mut Checker<'_>,
        mut value: Value,
        output: &Output,
        types: &[SourceType],
    ) -> Result<Value> {
        let opaque_input =
            output.input.is_some() && matches!(output.ty, SourceType::Generic { .. });
        value.sum = match output.sum {
            Some(id) => match checker.substitute(SourceType::Sum { id }, types)? {
                SourceType::Sum { id } => Some(id),
                _ => return Err(SourceCheckError::Type),
            },
            None if opaque_input => value.sum,
            None => None,
        };
        value.integer_domain = match output.integer_domain {
            Some(domain) => Some(domain),
            None if opaque_input => value.integer_domain,
            None => None,
        };
        value.structural = match output.structural {
            Some(ty) => Some(checker.substitute(ty, types)?),
            None if opaque_input => value.structural,
            None => None,
        };
        checker.scalar_facts(value)
    }

    fn attach(&self, checker: &mut Checker<'_>) -> Result<()> {
        assert_eq!(self.nodes.len(), self.values.len());
        for (output, value) in self.nodes.iter().zip(&self.values) {
            if output.input.is_some() {
                continue;
            }
            if let Some(owner) = value.owner {
                checker.charge(output.children.len())?;
                checker.proof.owners[owner].materialized = output.materialized;
                checker.charge(output.slots.len())?;
                checker.proof.owners[owner].slots = output.slots.clone();
                for child in &output.children {
                    checker.proof.owners[owner]
                        .children
                        .push(self.values[*child]);
                }
            }
        }
        // A record the callee built satisfied its own claims, so they are facts about the
        // children the caller now sees. Done after every output is attached, so a child is not
        // revealed early (and then attached twice).
        for (output, value) in self.nodes.iter().zip(&self.values) {
            if output.input.is_some() || !output.materialized || output.children.is_empty() {
                continue;
            }
            if let (SourceType::Aggregate { id }, Some(owner)) = (value.ty, value.owner) {
                let mut children = checker.proof.owners[owner].children.clone();
                match checker.field_requirements(id, &mut children, true) {
                    Err(SourceCheckError::Budget) => return Err(SourceCheckError::Budget),
                    _ => {}
                }
                // A dependent array the callee built (`data ref [element; len]`) is as long as
                // the length it was built with, as for an array handed in.
                checker.set_input_extents(id, &children);
            }
        }
        if let Some(owner) = self.values.first().and_then(|value| value.owner) {
            checker.obligate(owner);
        }
        Ok(())
    }
}
