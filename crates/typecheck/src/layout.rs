use crate::representation::{Repr, RepresentationError};
use crate::{TypeRegistry, ty::Ty};
use flask::{CompileTarget, TargetArch};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutTarget {
    X86_64,
    Arm64,
    Wasm32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbiClass {
    Empty,
    Scalar,
    Aggregate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutKind {
    Scalar,
    Address,
    Product {
        field_offsets: Vec<u64>,
    },
    Array {
        element: Box<Layout>,
        stride: u64,
        length: u64,
    },
    Sum {
        discriminant: Repr,
        payload_offset: u64,
        payload_size: u64,
        payload_alignment: u64,
        variant_layouts: Vec<Layout>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub size: u64,
    pub alignment: u64,
    pub kind: LayoutKind,
    pub abi: AbiClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LayoutError {
    #[error("target layout is unsupported")]
    UnsupportedTarget,
    #[error("{0}")]
    UnsupportedRepresentation(RepresentationError),
    #[error("aggregate layout size overflowed")]
    SizeOverflow,
    #[error("aggregate layout alignment overflowed")]
    AlignmentOverflow,
    #[error("raw-pointer pointee has zero size")]
    ZeroSizedPointee,
    #[error(
        "natural alignment {alignment} for {type_name} in address space {address_space} is not representable with index width {index_bits}; maximum supported alignment is {max_alignment}"
    )]
    NaturalAlignmentUnrepresentable {
        type_name: String,
        address_space: u32,
        alignment: u64,
        index_bits: u32,
        max_alignment: u64,
    },
}

impl LayoutError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedTarget => "unsupported-target-layout",
            Self::UnsupportedRepresentation(_) => "unsupported-type-representation",
            Self::SizeOverflow | Self::AlignmentOverflow => "aggregate-layout-overflow",
            Self::ZeroSizedPointee => "zero-sized-pointee-offset",
            Self::NaturalAlignmentUnrepresentable { .. } => {
                "target-layout-natural-alignment-unrepresentable"
            }
        }
    }

    pub fn diagnostic(&self) -> diagnostic::Diagnostic {
        diagnostic::Diagnostic::new(self.code(), self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetLayout {
    target: LayoutTarget,
    address_size: u64,
    address_alignment: u64,
    index_bits: u32,
}

impl TargetLayout {
    pub const fn new(target: LayoutTarget) -> Self {
        match target {
            LayoutTarget::X86_64 | LayoutTarget::Arm64 => Self {
                target,
                address_size: 8,
                address_alignment: 8,
                index_bits: 64,
            },
            LayoutTarget::Wasm32 => Self {
                target,
                address_size: 4,
                address_alignment: 4,
                index_bits: 32,
            },
        }
    }

    pub const fn x86_64() -> Self {
        Self::new(LayoutTarget::X86_64)
    }

    pub fn from_compile_target(target: &CompileTarget) -> Result<Self, LayoutError> {
        let CompileTarget::Concrete(triple) = target else {
            return Err(LayoutError::UnsupportedTarget);
        };
        let target = match triple.arch {
            TargetArch::X86_64 => LayoutTarget::X86_64,
            TargetArch::Arm64 => LayoutTarget::Arm64,
            TargetArch::Wasm32 => LayoutTarget::Wasm32,
        };
        Ok(Self::new(target))
    }

    pub const fn address_size(self) -> u64 {
        self.address_size
    }

    pub const fn address_alignment(self) -> u64 {
        self.address_alignment
    }

    pub const fn pointer_bits(self) -> u64 {
        self.address_size * 8
    }

    pub const fn index_bits(self) -> u32 {
        self.index_bits
    }

    pub fn max_alignment(self) -> u64 {
        1u64 << self.index_bits.saturating_sub(2)
    }

    pub fn with_address_layout(
        target: LayoutTarget,
        address_size: u64,
        address_alignment: u64,
        index_bits: u32,
    ) -> Result<Self, LayoutError> {
        let layout = Self {
            target,
            address_size,
            address_alignment,
            index_bits,
        };
        layout.validate_natural_alignment("address", 0, address_alignment)?;
        Ok(layout)
    }

    pub const fn target(self) -> LayoutTarget {
        self.target
    }

    pub fn layout_repr(&self, repr: &Repr) -> Result<Layout, LayoutError> {
        let layout = match repr {
            Repr::Bits { width } => {
                let size = u64::from(width.div_ceil(8));
                Ok(Layout {
                    size,
                    alignment: scalar_alignment(size),
                    kind: LayoutKind::Scalar,
                    abi: AbiClass::Scalar,
                })
            }
            Repr::Address { .. } => Ok(Layout {
                size: self.address_size,
                alignment: self.address_alignment,
                kind: LayoutKind::Address,
                abi: AbiClass::Scalar,
            }),
            Repr::Product { fields } => self.layout_product(fields),
            Repr::Sum { variants } => self.layout_sum(variants),
            Repr::Array { element, length } => self.layout_array(element, *length),
        }?;
        self.validate_natural_alignment("representation", 0, layout.alignment)?;
        Ok(layout)
    }

    pub fn layout_ty(
        &self,
        ty: &Ty,
        registry: Option<&TypeRegistry>,
    ) -> Result<Layout, LayoutError> {
        let repr = Repr::derive(ty, registry).map_err(LayoutError::UnsupportedRepresentation)?;
        self.layout_repr(&repr)
    }

    pub fn pointee_stride(
        &self,
        pointee: &Ty,
        registry: Option<&TypeRegistry>,
    ) -> Result<u64, LayoutError> {
        let layout = self.layout_ty(pointee, registry)?;
        if layout.size == 0 {
            return Err(LayoutError::ZeroSizedPointee);
        }
        Ok(layout.size)
    }

    pub fn query(
        &self,
        kind: ast::TargetQueryKind,
        operand: &Ty,
        registry: Option<&TypeRegistry>,
    ) -> Result<u64, LayoutError> {
        match kind {
            ast::TargetQueryKind::IndexBits => Ok(u64::from(self.index_bits)),
            ast::TargetQueryKind::Size => {
                self.layout_ty(operand, registry).map(|layout| layout.size)
            }
            ast::TargetQueryKind::Alignment => self
                .layout_ty(operand, registry)
                .map(|layout| layout.alignment),
        }
    }

    fn layout_product(&self, fields: &[Repr]) -> Result<Layout, LayoutError> {
        if fields.is_empty() {
            return Ok(Layout {
                size: 0,
                alignment: 1,
                kind: LayoutKind::Product {
                    field_offsets: Vec::new(),
                },
                abi: AbiClass::Empty,
            });
        }

        let mut offset = 0_u64;
        let mut alignment = 1_u64;
        let mut field_offsets = Vec::with_capacity(fields.len());
        for field in fields {
            let layout = self.layout_repr(field)?;
            alignment = alignment.max(layout.alignment);
            offset = align_up(offset, layout.alignment)?;
            field_offsets.push(offset);
            offset = offset
                .checked_add(layout.size)
                .ok_or(LayoutError::SizeOverflow)?;
        }
        let size = align_up(offset, alignment)?;
        Ok(Layout {
            size,
            alignment,
            kind: LayoutKind::Product { field_offsets },
            abi: AbiClass::Aggregate,
        })
    }

    fn layout_array(&self, element: &Repr, length: u64) -> Result<Layout, LayoutError> {
        let element = self.layout_repr(element)?;
        let stride = if element.size == 0 {
            0
        } else {
            align_up(element.size, element.alignment)?
        };
        let size = stride
            .checked_mul(length)
            .ok_or(LayoutError::SizeOverflow)?;
        Ok(Layout {
            size,
            alignment: if size == 0 { 1 } else { element.alignment },
            kind: LayoutKind::Array {
                element: Box::new(element),
                stride,
                length,
            },
            abi: if size == 0 {
                AbiClass::Empty
            } else {
                AbiClass::Aggregate
            },
        })
    }

    fn layout_sum(&self, variants: &[Repr]) -> Result<Layout, LayoutError> {
        let discriminant = Repr::Bits {
            width: discriminant_width(variants.len()),
        };
        let discriminant_layout = self.layout_repr(&discriminant)?;
        let variant_layouts = variants
            .iter()
            .map(|variant| self.layout_repr(variant))
            .collect::<Result<Vec<_>, _>>()?;
        let payload_size = variant_layouts
            .iter()
            .map(|layout| layout.size)
            .max()
            .unwrap_or(0);
        let payload_alignment = variant_layouts
            .iter()
            .map(|layout| layout.alignment)
            .max()
            .unwrap_or(1);
        let payload_offset = align_up(discriminant_layout.size, payload_alignment)?;
        let alignment = discriminant_layout.alignment.max(payload_alignment);
        let size = align_up(
            payload_offset
                .checked_add(payload_size)
                .ok_or(LayoutError::SizeOverflow)?,
            alignment,
        )?;
        Ok(Layout {
            size,
            alignment,
            kind: LayoutKind::Sum {
                discriminant,
                payload_offset,
                payload_size,
                payload_alignment,
                variant_layouts,
            },
            abi: if payload_size == 0 {
                AbiClass::Scalar
            } else {
                AbiClass::Aggregate
            },
        })
    }

    fn validate_natural_alignment(
        &self,
        type_name: &str,
        address_space: u32,
        alignment: u64,
    ) -> Result<(), LayoutError> {
        let max_alignment = 1_u64
            .checked_shl(self.index_bits.saturating_sub(2))
            .unwrap_or(u64::MAX);
        if self.index_bits < 2
            || alignment == 0
            || !alignment.is_power_of_two()
            || alignment > max_alignment
        {
            return Err(LayoutError::NaturalAlignmentUnrepresentable {
                type_name: type_name.to_string(),
                address_space,
                alignment,
                index_bits: self.index_bits,
                max_alignment,
            });
        }
        Ok(())
    }
}

impl Default for TargetLayout {
    fn default() -> Self {
        Self::x86_64()
    }
}

fn scalar_alignment(size: u64) -> u64 {
    size.next_power_of_two().clamp(1, 16)
}

fn discriminant_width(variant_count: usize) -> u32 {
    if variant_count == 2 {
        1
    } else if variant_count <= 256 {
        8
    } else if variant_count <= 65_536 {
        16
    } else if (variant_count as u64) <= u64::from(u32::MAX) + 1 {
        32
    } else {
        64
    }
}

fn align_up(value: u64, alignment: u64) -> Result<u64, LayoutError> {
    let remainder = value % alignment;
    if remainder == 0 {
        return Ok(value);
    }
    value
        .checked_add(alignment - remainder)
        .ok_or(LayoutError::AlignmentOverflow)
}
#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
