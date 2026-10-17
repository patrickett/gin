use crate::prelude::*;
use melior::ir::operation::OperationRef;

pub(super) struct FunctionStorage<'c> {
    entry: BlockRef<'c, 'c>,
    tail: Option<(Value<'c, 'c>, OperationRef<'c, 'c>)>,
}

impl<'c> FunctionStorage<'c> {
    pub(super) fn new(entry: BlockRef<'c, 'c>) -> Self {
        Self { entry, tail: None }
    }

    fn allocate(
        &mut self,
        context: &'c Context,
        ty: Type<'c>,
        location: Location<'c>,
    ) -> Value<'c, 'c> {
        let (count, last) = self.tail.unwrap_or_else(|| {
            let constant = arith_dialect::constant(
                context,
                IntegerAttribute::new(context.i64(), 1).into(),
                location,
            );
            let operation = self.entry.insert_operation(0, constant);
            (operation.result(0).unwrap().into(), operation)
        });
        let operation = melior::dialect::llvm::alloca(
            context,
            count,
            r#type::pointer(context, 0),
            location,
            melior::dialect::llvm::AllocaOptions::default().elem_type(Some(TypeAttribute::new(ty))),
        );
        let operation = self.entry.insert_operation_after(last, operation);
        self.tail = Some((count, operation));
        operation.result(0).unwrap().into()
    }
}

impl<'a, 'c> CodegenContext<'a, 'c> {
    pub(super) fn allocate_stack_slot(&self, ty: Type<'c>) -> Option<Value<'c, 'c>> {
        let mut storage = self.function_storage.borrow_mut();
        let Some(storage) = storage.as_mut() else {
            self.emit_internal("stack storage requested outside a function");
            return None;
        };
        Some(storage.allocate(self.mlir, ty, self.location()))
    }
}
