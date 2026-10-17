use super::EvalResult;
use ast::ConstValue;
use diagnostic::Diagnostic;
use diagnostic::code::EvaluationCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address {
    allocation: usize,
    offset: usize,
}

impl Address {
    pub fn offset(self, amount: isize) -> EvalResult<Self> {
        let offset = if amount.is_negative() {
            self.offset.checked_sub(amount.unsigned_abs())
        } else {
            self.offset.checked_add(amount as usize)
        };
        offset.map(|offset| Self { offset, ..self }).ok_or_else(|| {
            // compile-time-address-overflow
            Diagnostic::new(
                EvaluationCode::AddressOverflow,
                "compile-time pointer arithmetic overflowed the virtual address space",
            )
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct EvalHeap {
    allocations: Vec<Allocation>,
}

#[derive(Debug, Clone)]
enum Allocation {
    Cells(Vec<Option<ConstValue>>),
    Projection { base: Address },
    Aggregate(Vec<Address>),
    Reference(Address),
}

fn aggregate_item(value: &ConstValue, index: usize) -> Option<&ConstValue> {
    match value {
        ConstValue::FixedArray(items)
        | ConstValue::Sequence(items)
        | ConstValue::ResultRow(items) => items.get(index),
        ConstValue::Record { fields } => fields.get(index).map(|(_, value)| value),
        ConstValue::Tag { args, .. } => args.get(index),
        _ => None,
    }
}

fn aggregate_item_mut(value: &mut ConstValue, index: usize) -> Option<&mut ConstValue> {
    match value {
        ConstValue::FixedArray(items)
        | ConstValue::Sequence(items)
        | ConstValue::ResultRow(items) => std::sync::Arc::make_mut(items).get_mut(index),
        ConstValue::Record { fields } => std::sync::Arc::make_mut(fields)
            .get_mut(index)
            .map(|(_, value)| value),
        ConstValue::Tag { args, .. } => std::sync::Arc::make_mut(args).get_mut(index),
        _ => None,
    }
}

impl EvalHeap {
    fn push(&mut self, allocation: Allocation) -> Address {
        let address = Address {
            allocation: self.allocations.len(),
            offset: 0,
        };
        self.allocations.push(allocation);
        address
    }

    pub fn allocate(&mut self, value: ConstValue) -> Address {
        self.push(Allocation::Cells(vec![Some(value)]))
    }

    pub fn allocate_aggregate(&mut self, fields: Vec<Address>) -> Address {
        self.push(Allocation::Aggregate(fields))
    }

    pub fn allocate_reference(&mut self, referent: Address) -> Address {
        self.push(Allocation::Reference(referent))
    }

    pub fn reference(&self, address: Address) -> Option<Address> {
        match self.allocations.get(address.allocation)? {
            Allocation::Reference(referent) if address.offset == 0 => Some(*referent),
            _ => None,
        }
    }

    pub fn project(&mut self, base: Address, index: usize) -> EvalResult<Address> {
        if let Some(Allocation::Aggregate(fields)) = self.allocations.get(base.allocation) {
            return fields
                .get(index)
                .filter(|_| base.offset == 0)
                .copied()
                .ok_or_else(|| {
                    // compile-time-index-out-of-bounds
                    Diagnostic::new(
                        EvaluationCode::IndexOutOfBounds,
                        "projection is outside the initialized aggregate",
                    )
                });
        }
        if self
            .load(base)
            .and_then(|value| aggregate_item(value, index))
            .is_none()
        {
            // compile-time-index-out-of-bounds
            return Err(Diagnostic::new(
                EvaluationCode::IndexOutOfBounds,
                "projection is outside the initialized aggregate",
            ));
        }
        let address = self.push(Allocation::Projection { base });
        Ok(Address {
            offset: index,
            ..address
        })
    }

    pub fn slice(&mut self, base: Address, start: usize, length: usize) -> EvalResult<Address> {
        let extent = match self.allocations.get(base.allocation) {
            Some(Allocation::Aggregate(fields)) if base.offset == 0 => Some(fields.len()),
            _ => self.load(base).and_then(|value| match value {
                ConstValue::FixedArray(items) | ConstValue::Sequence(items) => Some(items.len()),
                _ => None,
            }),
        }
        .ok_or_else(|| {
            // compile-time-invalid-address
            Diagnostic::new(
                EvaluationCode::InvalidAddress,
                "slice source is not array storage",
            )
        })?;
        if start > extent || length > extent - start {
            // compile-time-index-out-of-bounds
            return Err(Diagnostic::new(
                EvaluationCode::IndexOutOfBounds,
                "slice exceeds its backing array",
            ));
        }
        let mut fields = Vec::with_capacity(length);
        for index in start..start + length {
            fields.push(self.project(base, index)?);
        }
        Ok(self.allocate_aggregate(fields))
    }

    pub fn load(&self, address: Address) -> Option<&ConstValue> {
        match self.allocations.get(address.allocation)? {
            Allocation::Cells(cells) => cells.get(address.offset)?.as_ref(),
            Allocation::Projection { base } => aggregate_item(self.load(*base)?, address.offset),
            Allocation::Aggregate(_) | Allocation::Reference(_) => None,
        }
    }

    fn load_mut(&mut self, address: Address) -> Option<&mut Option<ConstValue>> {
        match self.allocations.get_mut(address.allocation)? {
            Allocation::Cells(cells) => cells.get_mut(address.offset),
            Allocation::Projection { .. } | Allocation::Aggregate(_) | Allocation::Reference(_) => {
                None
            }
        }
    }

    pub fn store(&mut self, address: Address, value: ConstValue) -> EvalResult<()> {
        if let Some(Allocation::Projection { base }) = self.allocations.get(address.allocation) {
            let base = *base;
            let index = address.offset;
            let mut aggregate = self.freeze(base)?;
            let slot = aggregate_item_mut(&mut aggregate, index).ok_or_else(|| {
                // compile-time-index-out-of-bounds
                Diagnostic::new(
                    EvaluationCode::IndexOutOfBounds,
                    "projection is outside the aggregate",
                )
            })?;
            *slot = value;
            return self.store(base, aggregate);
        }
        let slot = self.load_mut(address).ok_or_else(|| {
            // compile-time-invalid-address
            Diagnostic::new(
                EvaluationCode::InvalidAddress,
                "compile-time evaluation accessed an invalid virtual address",
            )
        })?;
        *slot = Some(value);
        Ok(())
    }

    pub fn freeze(&self, address: Address) -> EvalResult<ConstValue> {
        self.load(address).cloned().ok_or_else(|| {
            // compile-time-invalid-address
            Diagnostic::new(
                EvaluationCode::InvalidAddress,
                "compile-time evaluation froze an invalid virtual address",
            )
        })
    }
}
