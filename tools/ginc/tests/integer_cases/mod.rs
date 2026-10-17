use std::cmp::Ordering;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Integer {
    Negative(u128),
    Nonnegative(u128),
}

impl Integer {
    pub(super) fn predecessor(self) -> Option<Self> {
        match self {
            Self::Negative(value) => value.checked_add(1).map(Self::Negative),
            Self::Nonnegative(0) => Some(Self::Negative(1)),
            Self::Nonnegative(value) => Some(Self::Nonnegative(value - 1)),
        }
    }

    pub(super) fn successor(self) -> Option<Self> {
        match self {
            Self::Negative(1) => Some(Self::Nonnegative(0)),
            Self::Negative(value) => Some(Self::Negative(value - 1)),
            Self::Nonnegative(value) => value.checked_add(1).map(Self::Nonnegative),
        }
    }

    pub(super) fn compare(self, other: Self) -> Ordering {
        match (self, other) {
            (Self::Negative(left), Self::Negative(right)) => right.cmp(&left),
            (Self::Nonnegative(left), Self::Nonnegative(right)) => left.cmp(&right),
            (Self::Negative(_), Self::Nonnegative(_)) => Ordering::Less,
            (Self::Nonnegative(_), Self::Negative(_)) => Ordering::Greater,
        }
    }
}

impl fmt::Display for Integer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Negative(magnitude) => write!(formatter, "-{magnitude}"),
            Self::Nonnegative(value) => write!(formatter, "{value}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Domain {
    pub(super) width: u32,
    pub(super) signed: bool,
}

impl Domain {
    pub(super) fn contains(self, value: Integer) -> bool {
        let (minimum, maximum) = self.bounds();
        value.compare(minimum) != Ordering::Less && value.compare(maximum) != Ordering::Greater
    }

    pub(super) fn contains_domain(self, source: Self) -> bool {
        let (minimum, maximum) = source.bounds();
        self.contains(minimum) && self.contains(maximum)
    }

    pub(super) fn symbol(self) -> String {
        format!("{}{}", if self.signed { "s" } else { "u" }, self.width)
    }

    pub(super) fn bounds(self) -> (Integer, Integer) {
        if self.signed {
            let limit = 1_u128 << (self.width - 1);
            (Integer::Negative(limit), Integer::Nonnegative(limit - 1))
        } else {
            let maximum = u128::MAX >> (128 - self.width);
            (Integer::Nonnegative(0), Integer::Nonnegative(maximum))
        }
    }

    pub(super) fn boundary_values(self) -> Vec<Integer> {
        let (minimum, maximum) = self.bounds();
        let Integer::Nonnegative(maximum_value) = maximum else {
            unreachable!()
        };
        let mut values = vec![
            minimum,
            Integer::Nonnegative(1),
            Integer::Nonnegative(maximum_value - 1),
            maximum,
        ];
        if let Integer::Negative(magnitude) = minimum {
            values.extend([
                Integer::Negative(magnitude - 1),
                Integer::Negative(1),
                Integer::Nonnegative(0),
            ]);
        } else {
            let sign_bit = 1_u128 << (self.width - 1);
            values.extend([
                Integer::Nonnegative(2),
                Integer::Nonnegative(sign_bit - 1),
                Integer::Nonnegative(sign_bit),
                Integer::Nonnegative(sign_bit + 1),
            ]);
        }
        values
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (minimum, maximum) = self.bounds();
        write!(formatter, "{minimum} to {maximum}")
    }
}

pub(super) fn domains() -> impl Iterator<Item = Domain> {
    [8, 16, 32, 64, 128]
        .into_iter()
        .flat_map(|width| [false, true].map(|signed| Domain { width, signed }))
}
