use codegen::{ContextExt, NativeCompiler, ScopedSymbolTable, Slot};
use internment::Intern;
use melior::ir::{Block, BlockLike, Location, Value};
use typecheck::ty::Ty;

#[test]
fn shadowed_bindings_restore_the_outer_scope_on_pop() {
    let context = NativeCompiler::create_context();
    let block = Block::new(&[(context.i64(), Location::unknown(&context)); 2]);
    let outer: Value = block.argument(0).unwrap().into();
    let inner: Value = block.argument(1).unwrap().into();
    let name = Intern::<String>::new("value".into());
    let mut symbols = ScopedSymbolTable::new();
    symbols.insert(name, outer, Ty::i64(), false);
    symbols.push_scope();
    symbols.insert(name, inner, Ty::i64(), true);
    assert_eq!(symbols.get_value(&name), Some(inner));
    assert_eq!(symbols.get(&name).unwrap().ty, Ty::i64());
    assert!(symbols.get(&name).unwrap().is_slot);

    symbols.pop_scope();
    assert_eq!(symbols.get_value(&name), Some(outer));
    assert!(!symbols.get(&name).unwrap().is_slot);
}

#[test]
fn branch_clones_do_not_share_assignments() {
    let context = NativeCompiler::create_context();
    let block = Block::new(&[(context.i64(), Location::unknown(&context)); 2]);
    let original: Value = block.argument(0).unwrap().into();
    let replacement: Value = block.argument(1).unwrap().into();
    let name = Intern::<String>::new("value".into());
    let absent = Intern::<String>::new("absent".into());
    let mut symbols = ScopedSymbolTable::new();
    symbols.insert(name, original, Ty::i64(), false);
    let mut branch = symbols.clone();
    let assigned = branch.assign(
        &name,
        Slot {
            value: replacement,
            ty: Ty::i64(),
            is_slot: true,
        },
    );
    assert!(assigned);
    assert_eq!(symbols.get_value(&name), Some(original));
    assert!(!symbols.get(&name).unwrap().is_slot);
    assert_eq!(branch.get_value(&name), Some(replacement));
    assert!(branch.get(&name).unwrap().is_slot);

    let assigned = branch.assign(
        &absent,
        Slot {
            value: replacement,
            ty: Ty::i64(),
            is_slot: false,
        },
    );
    assert!(!assigned);
}

#[test]
#[should_panic(expected = "Cannot pop the global scope")]
fn the_global_scope_cannot_be_removed() {
    ScopedSymbolTable::new().pop_scope();
}
