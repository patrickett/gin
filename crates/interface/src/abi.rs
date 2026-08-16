use std::collections::HashSet;

use crate::Fingerprint;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterMode {
    Own,
    Ref,
    Mut,
    Eat,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AbiParameter {
    pub name: String,
    pub mode: ParameterMode,
    pub default: Option<Fingerprint>,
    pub receiver: bool,
    pub foreign_slot: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AbiSignature {
    pub parameters: Vec<AbiParameter>,
    pub foreign: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AbiValidationError {
    DuplicateName(String),
    MissingForeignSlot(String),
    DuplicateForeignSlot(u32),
}

impl AbiSignature {
    pub fn canonical_parameters(&self) -> Result<Vec<&AbiParameter>, AbiValidationError> {
        let mut names = HashSet::new();
        let mut slots = HashSet::new();
        for parameter in &self.parameters {
            if !names.insert(parameter.name.as_str()) {
                return Err(AbiValidationError::DuplicateName(parameter.name.clone()));
            }
            if self.foreign {
                let Some(slot) = parameter.foreign_slot else {
                    return Err(AbiValidationError::MissingForeignSlot(
                        parameter.name.clone(),
                    ));
                };
                if !slots.insert(slot) {
                    return Err(AbiValidationError::DuplicateForeignSlot(slot));
                }
            }
        }
        let mut parameters: Vec<_> = self.parameters.iter().collect();
        if self.foreign {
            parameters.sort_by_key(|parameter| parameter.foreign_slot);
        } else {
            parameters.sort_by(|left, right| {
                right
                    .receiver
                    .cmp(&left.receiver)
                    .then_with(|| left.name.as_bytes().cmp(right.name.as_bytes()))
            });
        }
        Ok(parameters)
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, AbiValidationError> {
        let parameters = self.canonical_parameters()?;
        let mut output = vec![u8::from(self.foreign)];
        write_uleb(parameters.len(), &mut output);
        for parameter in parameters {
            write_string(&parameter.name, &mut output);
            output.push(parameter.mode as u8);
            output.push(u8::from(parameter.receiver));
            match parameter.default {
                Some(default) => {
                    output.push(1);
                    output.extend_from_slice(&default.0);
                }
                None => output.push(0),
            }
            if self.foreign {
                output.extend_from_slice(&parameter.foreign_slot.unwrap().to_le_bytes());
            }
        }
        Ok(output)
    }

    pub fn permute_evaluated_arguments<T>(
        &self,
        written: Vec<(String, T)>,
    ) -> Result<Vec<T>, AbiValidationError> {
        let canonical = self.canonical_parameters()?;
        let mut written: Vec<_> = written.into_iter().map(Some).collect();
        Ok(canonical
            .into_iter()
            .filter_map(|parameter| {
                let index = written.iter().position(|argument| {
                    argument
                        .as_ref()
                        .is_some_and(|(name, _)| name == &parameter.name)
                })?;
                written[index].take().map(|(_, value)| value)
            })
            .collect())
    }
}

fn write_string(value: &str, output: &mut Vec<u8>) {
    write_uleb(value.len(), output);
    output.extend_from_slice(value.as_bytes());
}

fn write_uleb(mut value: usize, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}
