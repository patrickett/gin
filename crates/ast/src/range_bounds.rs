//! Bounds helpers for `in N...M` scalar types and range-value tags.
//!
//! Provides [`InclusiveBounds`] and associated methods on [`Ty`] and [`DeclareValue`]
//! so that bound-related queries live on the types they inspect.

use i256::I256;
use internment::Intern;

use crate::declare::DeclareValue;
use crate::ty::Ty;

/// Inclusive range endpoints (closed interval `[min, max]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InclusiveBounds {
    pub min: i128,
    pub max: i128,
}

impl InclusiveBounds {
    pub fn from_i256(min: I256, max: I256) -> Option<Self> {
        let min = min.as_i128();
        let max = max.as_i128();
        if min > max {
            return None;
        }
        Some(Self { min, max })
    }

    pub fn contains(&self, value: i128) -> bool {
        value >= self.min && value <= self.max
    }

    /// `inner` ⊆ `outer` (subset for subtyping).
    pub fn contains_bounds(&self, inner: &InclusiveBounds) -> bool {
        self.min <= inner.min && inner.max <= self.max
    }
}

impl Ty {
    /// Extract inclusive scalar bounds from a resolved bounded-int type.
    pub fn int_bounds(&self) -> Option<InclusiveBounds> {
        let Ty::Int { min, max, .. } = self else {
            return None;
        };
        Some(InclusiveBounds {
            min: *min.as_ref()?,
            max: *max.as_ref()?,
        })
    }

    /// Known compile-time integer value from a type (literal / const-folded).
    pub fn int_known_value(&self) -> Option<i128> {
        match self {
            Ty::Int { value: Some(v), .. } => Some(*v),
            _ => None,
        }
    }

    /// Check a known value against this type's bounded-int expectations.
    ///
    /// Returns `Ok(())` when the value is within bounds, or `Err(bounds)` on
    /// a violation.
    pub fn check_int_in_bounds(&self, value: i128) -> Result<(), InclusiveBounds> {
        let bounds = self
            .int_bounds()
            .ok_or(InclusiveBounds { min: 0, max: 0 })?;
        if bounds.contains(value) {
            Ok(())
        } else {
            Err(bounds)
        }
    }

    /// Whether `self` is assignable to a parameter typed as `expected`
    /// (subset subtyping for bounded ints).
    pub fn int_assignable_to(&self, expected: &Ty) -> bool {
        match (self.int_bounds(), expected.int_bounds()) {
            (Some(a), Some(e)) => e.contains_bounds(&a),
            (None, Some(_)) => false,
            (_, None) => self == expected || self.fallback_equal_to(expected),
        }
    }

    /// Build a bounded scalar `Ty::Int` from `is in` / `is N...M` bounds.
    pub fn bounded_int(min: I256, max: I256) -> Ty {
        let width = Self::range_bit_width(min, max);
        let signed = min.is_negative();
        let (min, max) = (min.as_i128(), max.as_i128());
        Ty::Int {
            width,
            signed,
            value: None,
            min: Some(min),
            max: Some(max),
        }
    }

    /// Nominal range-value record (`WeekRange is 1...7`).
    pub fn range_value_record(name: Intern<String>, min: I256, max: I256) -> Ty {
        let elem = Ty::bounded_int(min, max);
        Ty::Record {
            name,
            resolved_params: None,
            fields: vec![
                (Intern::new("start".to_string()), Box::new(elem.clone())),
                (Intern::new("end".to_string()), Box::new(elem)),
            ],
        }
    }

    /// Scalar bounds for `name in WeekRange` when the type resolved to a range-value record.
    pub fn scalar_bounds_from_range_value(&self) -> Option<InclusiveBounds> {
        let Ty::Record { fields, .. } = self else {
            return None;
        };
        let start = fields
            .iter()
            .find(|(n, _)| n.as_str() == "start")
            .and_then(|(_, t)| t.int_bounds())?;
        let end = fields
            .iter()
            .find(|(n, _)| n.as_str() == "end")
            .and_then(|(_, t)| t.int_bounds())?;
        if start == end { Some(start) } else { None }
    }

    /// Whether this type is a range-value record (has `start`/`end` fields).
    pub fn is_range_value_record(&self) -> bool {
        matches!(self, Ty::Record { fields, .. } if fields.len() == 2
            && fields.iter().any(|(n, _)| n.as_str() == "start")
            && fields.iter().any(|(n, _)| n.as_str() == "end"))
    }

    /// Determine the smallest bit width that can represent `max - min`.
    fn range_bit_width(min: I256, max: I256) -> u8 {
        let range = max - min;
        if range <= I256::from_i128(i128::from(u8::MAX) + 1) {
            8
        } else if range <= I256::from_i128(i128::from(u16::MAX) + 1) {
            16
        } else if range <= I256::from_i128(i128::from(u32::MAX) + 1) {
            32
        } else if range <= I256::from_i128(i128::from(u64::MAX) + 1) {
            64
        } else {
            128
        }
    }
}

impl DeclareValue {
    /// Bounds from a tag declaration (`is in` or `is N...M`).
    pub fn bounds(&self) -> Option<InclusiveBounds> {
        match self {
            DeclareValue::InRange(start, end) | DeclareValue::Range(start, end) => {
                InclusiveBounds::from_i256(*start, *end)
            }
            _ => None,
        }
    }
}

impl Ty {
    /// Fallback equality check for ints without bounds (width + signed only).
    fn fallback_equal_to(&self, expected: &Ty) -> bool {
        match (self, expected) {
            (
                Ty::Int {
                    width: aw,
                    signed: as_,
                    ..
                },
                Ty::Int {
                    width: ew,
                    signed: es,
                    ..
                },
            ) => {
                aw == ew
                    && as_ == es
                    && self.int_bounds().is_none()
                    && expected.int_bounds().is_none()
            }
            _ => false,
        }
    }
}
