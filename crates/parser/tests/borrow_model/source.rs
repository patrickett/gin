use ast::{Convention, Item};

use super::model::{Error, Handle, Mode, Model};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceError {
    Syntax,
    Shape,
    UnwrittenConvention,
    OwnershipDeferred,
}

/// Extracts written conventions only. Types, groups, aliases, and effects are scenario inputs.
pub fn written_modes(source: &str) -> Result<Vec<Mode>, SourceError> {
    if source.len() > 4_096 {
        return Err(SourceError::Shape);
    }
    let file = parser::parse_strict(source).map_err(|_| SourceError::Syntax)?;
    let [item] = file.items.as_slice() else {
        return Err(SourceError::Shape);
    };
    let Item::Binding(binding) = &item.value else {
        return Err(SourceError::Shape);
    };
    let parameters = binding.parameters.as_ref().ok_or(SourceError::Shape)?;
    if parameters.value.len() > super::model::LIMIT {
        return Err(SourceError::Shape);
    }
    let mut modes = Vec::with_capacity(parameters.value.len());
    for parameter in &parameters.value {
        let convention = parameter
            .value
            .convention
            .as_ref()
            .ok_or(SourceError::UnwrittenConvention)?;
        let mode = match convention.value.keyword.value {
            Convention::Ref => Mode::Read,
            Convention::Mut => Mode::Write,
            Convention::Own | Convention::End => return Err(SourceError::OwnershipDeferred),
        };
        modes.push(mode);
    }
    Ok(modes)
}

/// A candidate composition rule, NOT a lowering of nested borrowed source syntax.
pub fn write_through_slot(model: &mut Model, slot: Handle) -> Result<(), Error> {
    let mut trial = model.clone();
    let target = trial.load_slot(slot)?;
    trial.write(target)?;
    *model = trial;
    Ok(())
}
