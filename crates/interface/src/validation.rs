use crate::{PublicInterface, codec};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InterfaceValidationError {
    NoncanonicalArtifact,
}

pub fn validate(interface: &PublicInterface) -> Result<(), InterfaceValidationError> {
    let bytes = codec::encode(interface);
    let decoded =
        codec::decode(&bytes).map_err(|_| InterfaceValidationError::NoncanonicalArtifact)?;
    if decoded == *interface {
        Ok(())
    } else {
        Err(InterfaceValidationError::NoncanonicalArtifact)
    }
}
