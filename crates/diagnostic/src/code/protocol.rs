use crate::code::traits::CodeDisplay;
use crate::code::{Code, Prefix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("{}", CodeDisplay(self))]
pub enum ProtocolCode {
    InputFailed,
    InvalidId,
    InvalidJson,
    InvalidRequest,
    RequestTooLarge,
    TaskTimeout,
}

impl Prefix for ProtocolCode {
    fn prefix(&self) -> &'static str {
        "jsonl"
    }
}

impl Code for ProtocolCode {
    fn name(&self) -> &'static str {
        match self {
            Self::InputFailed => "input-failed",
            Self::InvalidId => "invalid-id",
            Self::InvalidJson => "invalid-json",
            Self::InvalidRequest => "invalid-request",
            Self::RequestTooLarge => "request-too-large",
            Self::TaskTimeout => "task-timeout",
        }
    }
}

impl ProtocolCode {
    #[must_use]
    pub fn slug(self) -> String {
        self.to_string()
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::InputFailed,
            Self::InvalidId,
            Self::InvalidJson,
            Self::InvalidRequest,
            Self::RequestTooLarge,
            Self::TaskTimeout,
        ]
        .into_iter()
    }
}
