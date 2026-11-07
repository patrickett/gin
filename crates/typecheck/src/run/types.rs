//! Types as run time sees them: concrete, and independent of any module's instance arena.

use crate::SourceType;

/// A concrete type argument used when running an instantiated generic function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtType {
    Unit,
    Integer(SourceType),
    Nominal {
        home: String,
        name: String,
        args: Vec<RtType>,
    },
    Array {
        extent: u32,
        element: Box<RtType>,
    },
    Sequence(Vec<RtType>),
    Pointer(Box<RtType>),
}

impl RtType {
    pub fn spell(&self) -> String {
        let all = |types: &[RtType]| types.iter().map(Self::spell).collect::<Vec<_>>().join(", ");
        match self {
            Self::Unit => "()".to_owned(),
            Self::Integer(SourceType::Integer { lower, upper }) => format!("{lower} to {upper}"),
            Self::Integer(_) => "integer".to_owned(),
            Self::Nominal { name, args, .. } if args.is_empty() => name.clone(),
            Self::Nominal { name, args, .. } => format!("{name}({})", all(args)),
            Self::Array { extent, element } => format!("[{}; {extent}]", element.spell()),
            Self::Sequence(elements) => format!("seq({})", all(elements)),
            Self::Pointer(element) => format!("ptr({})", element.spell()),
        }
    }
}
