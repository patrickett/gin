//! `[value; count]`: an array of `count` copies of `value`.
//!
//! Copying is only free for an unrestricted element; a linear element would be duplicated along
//! with its obligation. The array is as long as the closed number `count` says. What it holds
//! is not tracked per slot: reading a slot yields an unknown value of the element type.
use super::*;

impl<'a> Checker<'a> {
    pub(super) fn repeat_tasks(
        &mut self,
        expression: &'a Expression,
        array: &'a ArrayExpression,
        expected: Option<SourceType>,
        visited: bool,
        tasks: &mut Vec<aggregates::EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        let [only] = array.elements.as_slice() else {
            return Err(SourceCheckError::Type);
        };
        let length = array.length.as_ref().ok_or(SourceCheckError::Malformed)?;
        let count = self.array_constant(&length.value)?;
        if count < I256::ZERO {
            return Err(SourceCheckError::Type);
        }
        if count > I256::from(u32::MAX) {
            return Err(SourceCheckError::Budget);
        }
        let extent = count.as_u32();
        let element = match expected {
            Some(ty) => {
                let (element, written) = self.array_layout(ty)?;
                if written != extent {
                    return Err(SourceCheckError::Type);
                }
                Some(element)
            }
            None => None,
        };
        if !visited {
            self.charge(2)?;
            tasks.push(aggregates::EvalTask::Expression(expression, expected, true));
            tasks.push(aggregates::EvalTask::Expression(only, element, false));
            return Ok(());
        }
        let mut child = values.pop().ok_or(SourceCheckError::Malformed)?;
        let element = match element {
            Some(ty) => ty,
            None => self.array_infer_element(std::slice::from_ref(&child))?,
        };
        child = self.view_as(child, element)?;
        if extent > 1 && self.capability_of(element) != Capability::Unrestricted {
            let detail = "repeating a value copies it, so its type must be `#unrestricted`".to_owned();
            return Err(self.explain(SourceCheckError::Type, detail));
        }
        self.release(child)?;
        let ty = self.array_type(element, extent)?;
        let value = self.fresh_value(ty)?;
        let owner = value.owner.ok_or(SourceCheckError::Type)?;
        self.proof.owners[owner].drained = extent == 0 || self.capability_of(element) == Capability::Unrestricted;
        self.obligate(owner);
        values.push(value);
        Ok(())
    }
}
