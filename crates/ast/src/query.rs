//! The kind of a compile-time `#` query.

use strum::{Display, EnumIter, EnumString, IntoStaticStr};

use crate::TargetQueryKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumIter, EnumString, IntoStaticStr)]
pub enum QueryKind {
    #[strum(serialize = "size")]
    Size,
    #[strum(serialize = "alignment")]
    Alignment,
    #[strum(serialize = "index_bits")]
    IndexBits,
    #[strum(serialize = "target")]
    Target,
}

impl TryFrom<QueryKind> for TargetQueryKind {
    type Error = ();

    fn try_from(kind: QueryKind) -> Result<Self, ()> {
        match kind {
            QueryKind::Size => Ok(Self::Size),
            QueryKind::Alignment => Ok(Self::Alignment),
            QueryKind::IndexBits => Ok(Self::IndexBits),
            QueryKind::Target => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::QueryKind;
    use crate::TargetQueryKind;
    use strum::IntoEnumIterator;

    #[test]
    fn derived_names_round_trip_through_display_and_from_str() {
        for kind in QueryKind::iter() {
            assert_eq!(kind.to_string(), <&str>::from(kind));
            assert_eq!(kind.to_string().parse(), Ok(kind));
        }
    }

    #[test]
    fn derived_names_are_snake_case() {
        assert_eq!(QueryKind::IndexBits.to_string(), "index_bits");
    }

    #[test]
    fn unknown_query_name_is_rejected() {
        assert_eq!(
            "size_of".parse::<QueryKind>(),
            Err(strum::ParseError::VariantNotFound)
        );
    }

    #[test]
    fn layout_kinds_convert_to_target_query_kinds() {
        for (kind, legacy) in [
            (QueryKind::Size, TargetQueryKind::Size),
            (QueryKind::Alignment, TargetQueryKind::Alignment),
            (QueryKind::IndexBits, TargetQueryKind::IndexBits),
        ] {
            assert_eq!(TargetQueryKind::try_from(kind), Ok(legacy));
        }
        assert_eq!(TargetQueryKind::try_from(QueryKind::Target), Err(()));
    }
}
