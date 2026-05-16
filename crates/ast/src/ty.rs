//! Type representation and layout calculations.

use crate::ConstValue;
use internment::Intern;

/// Resolved type — the canonical representation after resolving declared type names against declarations.
#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Int {
        width: u8,
        signed: bool,
        /// Known compile-time value when constant-folded, otherwise `None`.
        value: Option<i128>,
    },
    Float {
        /// Known compile-time value when constant-folded, otherwise `None`.
        value: Option<f64>,
    },
    Bool,
    Unit,
    Record {
        name: Intern<String>,
        fields: Vec<(Intern<String>, Box<Ty>)>,
    },
    Union {
        name: Intern<String>,
        /// Each variant: (variant_name, [(field_name, field_type)]) in declaration order.
        #[allow(clippy::type_complexity)]
        variants: Vec<(Intern<String>, Vec<(Intern<String>, Box<Ty>)>)>,
    },
    /// Unresolved / generic type — falls back to `i64` in codegen.
    Opaque(Intern<String>),
    /// Fixed-size stack-allocated array (`(T.new; N)`). The value is a `!llvm.ptr`.
    Array {
        elem: Box<Ty>,
        size: usize,
    },
    /// Raw pointer — erases `T` from layout, kept only for type checking. Maps to `!llvm.ptr`.
    Ptr {
        inner: Box<Ty>,
    },
    /// Reference (borrow-checked pointer in future). Same layout as `Ptr` for now.
    Ref {
        inner: Box<Ty>,
    },
    /// Positional tuple VALUE — used for tuple literals `(e1, e2, …)`. Maps to an LLVM struct.
    Tuple(Vec<Ty>),
    /// A closed set of compile-time-known literal values of a shared base type.
    /// Runtime representation is a small integer discriminant.
    ConstUnion {
        name: Intern<String>,
        /// The base type that all values inhabit (e.g. `str_record_ty()` for strings).
        base: Box<Ty>,
        /// The literal values in discriminant order.
        values: Vec<ConstValue>,
    },
}

impl Ty {
    /// Return record fields in layout order.
    ///
    /// Fields are sorted by descending alignment (then descending size, then declaration order
    /// for ties) so the compiler packs them without padding. The programmer writes fields in
    /// any logical order; the physical layout is determined here.
    /// Empty for non-record types.
    pub fn record_fields_sorted(&self) -> Vec<(&Intern<String>, &Ty)> {
        if let Ty::Record { fields, .. } = self {
            let mut indexed: Vec<(usize, &Intern<String>, &Ty)> = fields
                .iter()
                .enumerate()
                .map(|(i, (k, v))| (i, k, v.as_ref()))
                .collect();
            indexed.sort_by(|(i, _, a), (j, _, b)| {
                let align_a = ty_alignment(a);
                let align_b = ty_alignment(b);
                align_b
                    .cmp(&align_a)
                    .then_with(|| ty_byte_size_static(b).cmp(&ty_byte_size_static(a)))
                    .then_with(|| i.cmp(j))
            });
            indexed.into_iter().map(|(_, k, v)| (k, v)).collect()
        } else {
            vec![]
        }
    }

    pub fn is_int(&self) -> bool {
        matches!(self, Ty::Int { .. })
    }

    pub fn is_unsigned_int(&self) -> bool {
        matches!(self, Ty::Int { signed: false, .. })
    }

    pub fn is_signed_int(&self) -> bool {
        matches!(self, Ty::Int { signed: true, .. })
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Ty::Float { .. })
    }

    pub fn is_ptr_or_ref(&self) -> bool {
        matches!(self, Ty::Ptr { .. } | Ty::Ref { .. })
    }

    pub fn is_union(&self) -> bool {
        matches!(self, Ty::Union { .. })
    }

    pub fn is_const_union(&self) -> bool {
        matches!(self, Ty::ConstUnion { .. })
    }

    pub fn is_record(&self) -> bool {
        matches!(self, Ty::Record { .. })
    }

    /// Convenience constructor for a default-width signed integer type.
    pub fn i64() -> Self {
        Ty::Int {
            width: 64,
            signed: true,
            value: None,
        }
    }

    /// Convenience constructor for an 8-bit unsigned integer type.
    pub fn u8() -> Self {
        Ty::Int {
            width: 8,
            signed: false,
            value: None,
        }
    }
}

/// Type alias for union variant fields: (field_name, field_type)
#[allow(dead_code)]
pub(crate) type UnionFields = Vec<(Intern<String>, Box<Ty>)>;

/// Type alias for union variants: (variant_name, fields)
#[allow(dead_code)]
pub(crate) type UnionVariants = Vec<(Intern<String>, UnionFields)>;

/// Type alias for union variant fields: (variant_name, [(field_name, field_type)])
type UnionVariant<'a> = (Intern<String>, Vec<(Intern<String>, Box<Ty>)>);

// Layout / size utilities

pub fn ty_union_discriminant_size(num_variants: usize) -> usize {
    if num_variants <= 256 {
        1
    } else if num_variants <= 65536 {
        2
    } else {
        8
    }
}

pub(crate) fn ty_union_max_field_size(variants: &[UnionVariant<'_>]) -> usize {
    variants
        .iter()
        .flat_map(|(_, fields)| fields.iter().map(|(_, ft)| ty_byte_size_static(ft)))
        .max()
        .unwrap_or(0)
}

pub fn ty_alignment(ty: &Ty) -> usize {
    match ty {
        Ty::Int { width: 8, .. } | Ty::Bool => 1,
        Ty::Int { width: 16, .. } => 2,
        Ty::Int { width: 32, .. } => 4,
        Ty::Int { width: 128, .. } => 16,
        Ty::Int { .. } | Ty::Float { .. } | Ty::Array { .. } | Ty::Ptr { .. } | Ty::Ref { .. } => 8,
        Ty::Unit => 1,
        Ty::Opaque(_) => 8,
        Ty::Record { fields, .. } => fields
            .iter()
            .map(|(_, ft)| ty_alignment(ft))
            .max()
            .unwrap_or(1),
        Ty::Union { variants, .. } => {
            let all_empty = variants.iter().all(|(_, fields)| fields.is_empty());
            if all_empty {
                1
            } else {
                variants
                    .iter()
                    .flat_map(|(_, fields)| fields.iter().map(|(_, ft)| ty_alignment(ft)))
                    .max()
                    .unwrap_or(8)
            }
        }
        Ty::ConstUnion { .. } => 1,
        Ty::Tuple(fields) => fields.iter().map(ty_alignment).max().unwrap_or(1),
    }
}

/// Returns the in-memory size (bytes) of a type without recursing into the typeck context.
pub fn ty_byte_size_static(ty: &Ty) -> usize {
    match ty {
        Ty::Int { width: 8, .. } | Ty::Bool => 1,
        Ty::Int { width: 16, .. } => 2,
        Ty::Int { width: 32, .. } => 4,
        Ty::Int { width: 128, .. } => 16,
        Ty::Int { .. } | Ty::Float { .. } => 8,
        Ty::Array { .. } | Ty::Ptr { .. } | Ty::Ref { .. } => 8,
        Ty::Unit => 0,
        Ty::Opaque(_) => 8,
        Ty::Record { fields, .. } => fields.iter().map(|(_, ft)| ty_byte_size_static(ft)).sum(),
        Ty::Union { variants, .. } => {
            let all_empty = variants.iter().all(|(_, fields)| fields.is_empty());
            if all_empty && variants.len() <= 256 {
                1
            } else if all_empty {
                ty_union_discriminant_size(variants.len())
            } else {
                let discriminant_size = ty_union_discriminant_size(variants.len());
                let max_field_size = ty_union_max_field_size(variants);
                discriminant_size + max_field_size
            }
        }
        Ty::ConstUnion { values, .. } => {
            if values.len() <= 256 {
                1
            } else {
                ty_union_discriminant_size(values.len())
            }
        }
        Ty::Tuple(fields) => fields.iter().map(ty_byte_size_static).sum(),
    }
}

impl std::hash::Hash for Ty {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Ty::Int {
                width,
                signed,
                value,
            } => {
                width.hash(state);
                signed.hash(state);
                value.hash(state);
            }
            Ty::Float { value } => {
                // Hash raw bits so NaN values with different bit patterns are distinct.
                value.map(f64::to_bits).hash(state);
            }
            Ty::Bool => {}
            Ty::Unit => {}
            Ty::Record { name, fields } => {
                name.hash(state);
                fields.hash(state);
            }
            Ty::Union { name, variants } => {
                name.hash(state);
                variants.hash(state);
            }
            Ty::Opaque(name) => name.hash(state),
            Ty::Array { elem, size } => {
                elem.hash(state);
                size.hash(state);
            }
            Ty::Ptr { inner } => inner.hash(state),
            Ty::Ref { inner } => inner.hash(state),
            Ty::Tuple(fields) => fields.hash(state),
            Ty::ConstUnion { name, base, values } => {
                name.hash(state);
                base.hash(state);
                values.hash(state);
            }
        }
    }
}

impl Eq for Ty {}

/// Canonical `Str` record type: `{ pointer: Ptr(Byte), len: Int }`.
static STR_RECORD_TY: std::sync::OnceLock<Ty> = std::sync::OnceLock::new();

#[allow(unsafe_code)]
unsafe impl salsa::Update for Ty {
    unsafe fn maybe_update(old_pointer: *mut Self, new_value: Self) -> bool {
        let old_ref: &mut Self = unsafe { &mut *old_pointer };
        if *old_ref != new_value {
            *old_ref = new_value;
            true
        } else {
            false
        }
    }
}

pub fn str_record_ty() -> Ty {
    STR_RECORD_TY
        .get_or_init(|| Ty::Record {
            name: Intern::<String>::from_ref("Str"),
            fields: vec![
                (
                    Intern::<String>::from_ref("pointer"),
                    Box::new(Ty::Ptr {
                        inner: Box::new(Ty::Int {
                            width: 8,
                            signed: false,
                            value: None,
                        }),
                    }),
                ),
                (
                    Intern::<String>::from_ref("len"),
                    Box::new(Ty::Int {
                        width: 64,
                        signed: false,
                        value: None,
                    }),
                ),
            ],
        })
        .clone()
}
