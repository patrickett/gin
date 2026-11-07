//! `[a: x, b: y]`: a labeled array builds a record.
//!
//! Its type is the expected record when one is known (an annotation, a parameter, a field), else
//! the one record whose field names are exactly the labels, else an anonymous record of those
//! fields. Labels name fields, so the order written does not matter; every field is given once.
use super::*;
use aggregates::EvalTask;
use arrays::array_element_view;

impl<'a> Checker<'a> {
    pub(super) fn labeled_tasks(
        &mut self,
        expression: &'a Expression,
        array: &'a ArrayExpression,
        expected: Option<SourceType>,
        visited: bool,
        tasks: &mut Vec<EvalTask<'a>>,
        values: &mut Vec<Value>,
    ) -> Result<()> {
        if array.length.is_some() || array.labels.iter().any(Option::is_none) {
            return Err(SourceCheckError::Type);
        }
        let labels: Vec<&'a Name> = array.labels.iter().flatten().collect();
        for label in &labels {
            self.name(label)?;
        }
        let target = self.labeled_target(&labels, expected)?;
        if visited {
            let count = array.elements.len();
            if values.len() < count {
                return Err(SourceCheckError::Malformed);
            }
            let children = values.split_off(values.len() - count);
            let packed = match target {
                Some((schema, context, indices)) => {
                    self.pack_indexed(schema, context, indices, children)?
                }
                None => {
                    let fields = labels
                        .iter()
                        .zip(&children)
                        .map(|(label, child)| (label.value.clone(), array_element_view(*child)))
                        .collect();
                    let key = array as *const ArrayExpression as usize;
                    let stable = self.labeled_record_key();
                    let ty = self.anonymous_record_keyed(key, "labeled".to_owned(), stable, fields)?;
                    let SourceType::Aggregate { id } = ty else {
                        return Err(SourceCheckError::Type);
                    };
                    let schema = self.module.instances[id as usize].schema();
                    let indices = (0..count).collect();
                    self.pack_indexed(schema, Some(ty), indices, children)?
                }
            };
            values.push(packed);
            return Ok(());
        }
        self.charge(array.elements.len() + 1)?;
        tasks.push(EvalTask::Expression(expression, expected, true));
        let types = match &target {
            Some((schema, context, _)) => {
                // A record without type parameters has one instance.
                let context = match context {
                    None if self.module.layouts[*schema].schema.parameters.is_empty() => {
                        Some(self.instance(*schema, Vec::new())?)
                    }
                    other => *other,
                };
                match context {
                    Some(SourceType::Aggregate { id }) => Some(self.instance_fields(id)?),
                    _ => None,
                }
            }
            None => None,
        };
        for (position, child) in array.elements.iter().enumerate().rev() {
            let wanted = match (&target, &types) {
                (Some((_, _, indices)), Some(types)) => Some(types[indices[position]]),
                _ => None,
            };
            tasks.push(EvalTask::Expression(child, wanted, false));
        }
        Ok(())
    }

    /// The record the labels build and the field each names, or `None` for an anonymous record.
    #[allow(clippy::type_complexity)]
    fn labeled_target(
        &mut self,
        labels: &[&'a Name],
        expected: Option<SourceType>,
    ) -> Result<Option<(usize, Option<SourceType>, Vec<usize>)>> {
        let named = |checker: &mut Self, schema: usize| -> Result<Option<Vec<usize>>> {
            let count = checker.module.layouts[schema].schema.fields.len();
            checker.charge_product(count + 1, labels.len() + 1)?;
            let fields = &checker.module.layouts[schema].schema.fields;
            if fields.len() != labels.len()
                || checker.module.layouts[schema].schema.kind != AggregateKind::Record
            {
                return Ok(None);
            }
            let mut indices = Vec::with_capacity(labels.len());
            for label in labels {
                match fields.iter().position(|field| field.name == label.value) {
                    Some(index) if !indices.contains(&index) => indices.push(index),
                    _ => return Ok(None),
                }
            }
            Ok(Some(indices))
        };
        if let Some(SourceType::Aggregate { id }) = expected {
            let schema = self.module.instances[id as usize].schema();
            if self.is_anonymous_record(SourceType::Aggregate { id }) {
                return Ok(None);
            }
            let indices = named(self, schema)?.ok_or(SourceCheckError::Type)?;
            return Ok(Some((schema, expected, indices)));
        }
        let mut found = None;
        for schema in 0..self.module.layouts.len() {
            if schema >= self.module.schemas.len() && schema < self.module.layouts.len() {
                // An anonymous record is never the answer to a written label set.
                if self.module.anonymous_records.iter().any(|(_, layout)| *layout == schema) {
                    continue;
                }
            }
            if let Some(indices) = named(self, schema)? {
                if found.is_some() {
                    let detail = "more than one record has exactly these fields".to_owned();
                    return Err(self.explain(SourceCheckError::Duplicate, detail));
                }
                found = Some((schema, None, indices));
            }
        }
        Ok(found)
    }
}
