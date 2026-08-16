use crate::{InterfaceDecodeError, PublicInterface};

pub fn encode(interface: &PublicInterface) -> Vec<u8> {
    interface.canonical_bytes()
}

pub fn decode(bytes: &[u8]) -> Result<PublicInterface, InterfaceDecodeError> {
    PublicInterface::decode(bytes)
}
