use super::EvalResult;
use ast::ConstValue;
use diagnostic::Diagnostic;

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
            Diagnostic::new(
                "compile-time-address-overflow",
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
    pub fn allocate(&mut self, value: ConstValue) -> Address {
        let address = Address {
            allocation: self.allocations.len(),
            offset: 0,
        };
        self.allocations.push(Allocation::Cells(vec![Some(value)]));
        address
    }

    pub fn allocate_sequence(&mut self, values: impl IntoIterator<Item = ConstValue>) -> Address {
        let address = Address {
            allocation: self.allocations.len(),
            offset: 0,
        };
        self.allocations
            .push(Allocation::Cells(values.into_iter().map(Some).collect()));
        address
    }

    pub fn project(&mut self, base: Address, index: usize) -> EvalResult<Address> {
        if self
            .load(base)
            .and_then(|value| aggregate_item(value, index))
            .is_none()
        {
            return Err(Diagnostic::new(
                "compile-time-index-out-of-bounds",
                "projection is outside the initialized aggregate",
            ));
        }
        let address = Address {
            allocation: self.allocations.len(),
            offset: index,
        };
        self.allocations.push(Allocation::Projection { base });
        Ok(address)
    }

    pub fn load(&self, address: Address) -> Option<&ConstValue> {
        match self.allocations.get(address.allocation)? {
            Allocation::Cells(cells) => cells.get(address.offset)?.as_ref(),
            Allocation::Projection { base } => aggregate_item(self.load(*base)?, address.offset),
        }
    }

    fn load_mut(&mut self, address: Address) -> Option<&mut Option<ConstValue>> {
        match self.allocations.get_mut(address.allocation)? {
            Allocation::Cells(cells) => cells.get_mut(address.offset),
            Allocation::Projection { .. } => None,
        }
    }

    pub fn load_offset(&self, address: Address, offset: isize) -> EvalResult<&ConstValue> {
        let address = address.offset(offset)?;
        self.load(address).ok_or_else(|| {
            Diagnostic::new(
                "compile-time-invalid-address",
                "compile-time evaluation accessed an invalid virtual address",
            )
        })
    }

    pub fn store(&mut self, address: Address, value: ConstValue) -> EvalResult<()> {
        if let Some(Allocation::Projection { base }) = self.allocations.get(address.allocation) {
            let base = *base;
            let index = address.offset;
            let mut aggregate = self.freeze(base)?;
            let slot = aggregate_item_mut(&mut aggregate, index).ok_or_else(|| {
                Diagnostic::new(
                    "compile-time-index-out-of-bounds",
                    "projection is outside the aggregate",
                )
            })?;
            *slot = value;
            return self.store(base, aggregate);
        }
        let slot = self.load_mut(address).ok_or_else(|| {
            Diagnostic::new(
                "compile-time-invalid-address",
                "compile-time evaluation accessed an invalid virtual address",
            )
        })?;
        *slot = Some(value);
        Ok(())
    }

    pub fn store_offset(
        &mut self,
        address: Address,
        offset: isize,
        value: ConstValue,
    ) -> EvalResult<()> {
        self.store(address.offset(offset)?, value)
    }

    pub fn freeze(&self, address: Address) -> EvalResult<ConstValue> {
        self.load(address).cloned().ok_or_else(|| {
            Diagnostic::new(
                "compile-time-invalid-address",
                "compile-time evaluation froze an invalid virtual address",
            )
        })
    }
}
