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
        let hull = self.anonymous_integer_validity()?.domain().storage_hull()?;
        if hull.min() < I256::from(i128::MIN) || hull.max() > I256::from(i128::MAX) {
            return None;
        }
        Some(InclusiveBounds {
            min: hull.min().as_i128(),
            max: hull.max().as_i128(),
        })
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
        if let (Some(actual), Some(expected)) = (self.type_id(), expected.type_id())
            && actual != expected
        {
            return false;
        }
        match (self.int_bounds(), expected.int_bounds()) {
            (Some(a), Some(e)) => e.contains_bounds(&a),
            (None, Some(_)) => false,
            (_, None) => self == expected,
        }
    }

    /// Build a bounded anonymous integer from `is in` / `is N...M` bounds.
    pub fn bounded_int(min: I256, max: I256) -> Ty {
        let validity = crate::integer::IntegerDomain::bounded(min, max)
            .unwrap_or_else(crate::integer::IntegerDomain::empty);
        Ty::AnonymousInteger {
            validity: crate::integer::IntegerValidity::new(validity),
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
}

impl DeclareValue {
    /// Bounds from a tag declaration (`is in` or `is N...M`).
    pub fn bounds(&self) -> Option<InclusiveBounds> {
        match self {
            DeclareValue::InRange(start, end) | DeclareValue::Range(start, end) => {
                InclusiveBounds::from_i256(*start, *end)
            }
            DeclareValue::Refinement(_) => None,
            _ => None,
        }
    }
}
