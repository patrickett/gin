//! Type diagnostic variant.

use crate::diagnostic::{Category, Symptom, SymptomDetail, SymptomSource};
use chumsky::span::SimpleSpan;

pub enum TypeSymptom {
    Mismatch,
    UnknownBinding {
        name: String,
    },
    UnknownTag {
        name: String,
    },
    InferenceFailed,
    ConstraintViolation {
        param: String,
        expected: String,
        got: String,
    },
    UnresolvedTypeParam {
        name: String,
    },
    ArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },
    IndexOutOfBounds {
        index: i128,
        size: usize,
    },
    UnusedBinding {
        name: String,
    },
    NotAVariant {
        name: String,
        union_name: String,
    },
}

impl SymptomDetail for TypeSymptom {
    fn id(&self) -> u8 {
        match self {
            TypeSymptom::Mismatch => 1,
            TypeSymptom::UnknownBinding { .. } => 2,
            TypeSymptom::UnknownTag { .. } => 9,
            TypeSymptom::InferenceFailed => 3,
            TypeSymptom::ConstraintViolation { .. } => 4,
            TypeSymptom::UnresolvedTypeParam { .. } => 5,
            TypeSymptom::ArityMismatch { .. } => 6,
            TypeSymptom::IndexOutOfBounds { .. } => 7,
            TypeSymptom::UnusedBinding { .. } => 8,
            TypeSymptom::NotAVariant { .. } => 10,
        }
    }

    fn message(&self) -> String {
        match self {
            TypeSymptom::Mismatch => "type mismatch".into(),
            TypeSymptom::UnknownBinding { name } => format!("use of undefined binding `{name}`"),
            TypeSymptom::UnknownTag { name } => format!("use of undeclared tag `{name}`"),
            TypeSymptom::InferenceFailed => "failed to infer type".into(),
            TypeSymptom::ConstraintViolation {
                param,
                expected,
                got,
            } => {
                format!("type parameter `{param}` requires `{expected}`, got `{got}`")
            }
            TypeSymptom::UnresolvedTypeParam { name } => {
                format!("unresolved type parameter `{name}`")
            }
            TypeSymptom::ArityMismatch {
                name,
                expected,
                got,
            } => {
                format!("`{name}` expects {expected} type argument(s), got {got}")
            }
            TypeSymptom::IndexOutOfBounds { index, size } => {
                format!("index out of bounds: the len is {size} but the index is {index}")
            }
            TypeSymptom::UnusedBinding { name } => format!("unused binding `{name}`"),
            TypeSymptom::NotAVariant { name, union_name } => {
                format!("`{name}` is not a variant of `{union_name}`")
            }
        }
    }

    fn help(&self) -> Option<String> {
        match self {
            TypeSymptom::Mismatch => Some("types do not match".into()),
            TypeSymptom::UnknownBinding { .. } => Some("define bind before using it".into()),
            TypeSymptom::UnknownTag { .. } => Some("declare the tag before using it".into()),
            TypeSymptom::InferenceFailed => Some("could not infer the type".into()),
            TypeSymptom::ConstraintViolation {
                param, expected, ..
            } => Some(format!(
                "ensure the type argument for `{param}` satisfies the `{expected}` constraint"
            )),
            TypeSymptom::UnresolvedTypeParam { name } => Some(format!(
                "provide a concrete type for `{name}` at the instantiation site"
            )),
            TypeSymptom::ArityMismatch { expected, .. } => {
                Some(format!("provide exactly {expected} type argument(s)"))
            }
            TypeSymptom::IndexOutOfBounds { size, .. } => {
                Some(format!("valid indices are 0..{}", size))
            }
            TypeSymptom::UnusedBinding { .. } => None,
            TypeSymptom::NotAVariant { union_name, .. } => Some(format!(
                "expected one of the variants declared in `{union_name}`"
            )),
        }
    }
}

pub const fn mismatch(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::Mismatch),
        span,
        category: Category::Flaw,
    }
}

pub fn unknown_binding(span: SimpleSpan, name: String) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::UnknownBinding { name }),
        span,
        category: Category::Flaw,
    }
}

pub fn unknown_tag(span: SimpleSpan, name: String) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::UnknownTag { name }),
        span,
        category: Category::Flaw,
    }
}

pub const fn inference_failed(span: SimpleSpan) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::InferenceFailed),
        span,
        category: Category::Flaw,
    }
}

pub fn constraint_violation(
    span: SimpleSpan,
    param: String,
    expected: String,
    got: String,
) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::ConstraintViolation {
            param,
            expected,
            got,
        }),
        span,
        category: Category::Flaw,
    }
}

pub fn unresolved_type_param(span: SimpleSpan, name: String) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::UnresolvedTypeParam { name }),
        span,
        category: Category::Flaw,
    }
}

pub fn arity_mismatch(span: SimpleSpan, name: String, expected: usize, got: usize) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::ArityMismatch {
            name,
            expected,
            got,
        }),
        span,
        category: Category::Flaw,
    }
}

pub fn index_out_of_bounds(span: SimpleSpan, index: i128, size: usize) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::IndexOutOfBounds { index, size }),
        span,
        category: Category::Flaw,
    }
}

pub fn unused_binding(span: SimpleSpan, name: String) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::UnusedBinding { name }),
        span,
        category: Category::Help,
    }
}

pub fn not_a_variant(span: SimpleSpan, name: String, union_name: String) -> Symptom {
    Symptom {
        source: SymptomSource::Type(TypeSymptom::NotAVariant { name, union_name }),
        span,
        category: Category::Flaw,
    }
}
