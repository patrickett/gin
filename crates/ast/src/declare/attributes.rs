use crate::AttributeItem;

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct DeclareAttributes {
    /// Raw parsed attributes before semantic extraction.
    /// `None` means no `#[...]` block was present at all.
    /// `Some(vec![])` means an empty `#[]` was present.
    pub raw_attributes: Option<Vec<AttributeItem>>,
}

impl DeclareAttributes {
    /// Extract compiler-known intrinsic attributes from `raw_attributes` into typed fields.
    /// Should be called after parsing.
    pub fn extract_intrinsic_attributes(&mut self) {
        let Some(items) = &self.raw_attributes else {
            return;
        };
        if items.is_empty() {
            return;
        }

        for item in items {
            if let AttributeItem::Call {
                name: _, args: _, ..
            } = item
            {
                // No intrinsic call attributes for declares currently.
            } else if let AttributeItem::Flag { name: _, .. } = item {
            }
        }
    }
}
