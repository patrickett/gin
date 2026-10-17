use super::*;
use ast::prelude::Intern;
use parser::query::SourceParseExt;

fn bindings() -> HashMap<DefId, TypedBind> {
    let ast = "first(x, y): return 1\nsecond(x, y): return 2\nplain(x, y): return 3\n"
        .parse_source_full()
        .ast;
    let mut typed = crate::transform::stage_declare(
        &ast,
        crate::FileId(0),
        &crate::transform::TransformCtx::new(),
    );
    for (id, bind) in &mut typed.defs {
        bind.return_type = Ty::i64();
        bind.params = vec![
            (Intern::from_ref("x"), Ty::i64()),
            (Intern::from_ref("y"), Ty::i64()),
        ]
        .into();
        bind.infix_symbol = match id.0.as_str() {
            "first" => Some(Intern::from_ref("+")),
            "second" => Some(Intern::from_ref("-")),
            _ => None,
        };
    }
    typed.defs
}

#[test]
fn operator_table_is_undemanded_and_local_headers_are_snapshots() {
    let mut local = bindings();
    let shared = HashMap::new();
    let registry = OperatorRegistry::with_shared_signatures(&local, &shared);
    assert!(registry.entries.get().is_none());
    assert_eq!(
        registry
            .local
            .iter()
            .filter(|(_, header)| header.is_some())
            .count(),
        2
    );
    let id = DefId::named(Intern::from_ref("first"));
    let expected = TypedCallableSignature::from(&local[&id]);
    local.get_mut(&id).unwrap().return_type = Ty::u8();
    assert_eq!(
        registry.resolve("+", &Ty::i64(), &Ty::i64()),
        Ok((id, expected))
    );
    assert!(registry.entries.get().is_some());
}

#[test]
fn non_operator_local_declarations_still_shadow_shared_operators() {
    let mut local = bindings();
    let id = DefId::named(Intern::from_ref("plain"));
    let mut signature = TypedCallableSignature::from(&local[&id]);
    signature.infix_symbol = Some(Intern::from_ref("*"));
    let shared = HashMap::from([(id, signature)]);
    local.retain(|key, _| *key == id);
    let registry = OperatorRegistry::with_shared_signatures(&local, &shared);
    assert_eq!(
        registry.resolve("*", &Ty::i64(), &Ty::i64()),
        Err(OperatorResolutionError::NoVisibleContract)
    );
    assert!(registry.entries().is_empty());
}

#[test]
fn shared_headers_are_borrowed_and_results_are_owned() {
    let mut local = bindings();
    let id = DefId::named(Intern::from_ref("first"));
    let signature = TypedCallableSignature::from(&local.remove(&id).unwrap());
    let shared = HashMap::from([(id, signature.clone())]);
    let registry = OperatorRegistry::with_shared_signatures(&local, &shared);
    let (_, source) = registry
        .entries()
        .iter()
        .find(|(key, _)| *key == id)
        .unwrap();
    assert!(std::ptr::eq(
        registry.signature(source).unwrap(),
        &shared[&id]
    ));
    let (_, mut resolved) = registry.resolve("+", &Ty::i64(), &Ty::i64()).unwrap();
    resolved.return_type = Ty::u8();
    assert_eq!(shared[&id], signature);
}

#[test]
fn lazy_table_preserves_original_order_duplicate_rules_and_resolution() {
    for ordinal in [0, 1, 2] {
        let mut local = bindings();
        let id = DefId::named(Intern::from_ref("first"));
        let overloaded = DefId::member(id.0, ordinal);
        let original = local[&id].clone();
        local.insert(overloaded, original.clone());
        let mut shared = HashMap::from([
            (id, TypedCallableSignature::from(&original)),
            (overloaded, TypedCallableSignature::from(&original)),
        ]);
        let extra = DefId::named(Intern::from_ref("other"));
        shared.insert(extra, TypedCallableSignature::from(&original));
        let mut expected: Vec<_> = local
            .iter()
            .map(|(id, bind)| (*id, TypedCallableSignature::from(bind)))
            .chain(shared.iter().map(|(id, header)| (*id, header.clone())))
            .collect();
        let eager = OperatorRegistry::new(expected.clone().into_iter(), std::iter::empty());
        expected.sort_by(|(left, _), (right, _)| left.0.as_str().cmp(right.0.as_str()));
        expected.dedup_by_key(|(id, _)| *id);
        expected.retain(|(_, signature)| signature.infix_symbol.is_some());
        let registry = OperatorRegistry::with_shared_signatures(&local, &shared);
        assert_eq!(
            registry
                .entries()
                .iter()
                .map(|(id, _)| *id)
                .collect::<Vec<_>>(),
            expected.iter().map(|(id, _)| *id).collect::<Vec<_>>()
        );
        for (entry, (id, header)) in registry.entries().iter().zip(&expected) {
            assert_eq!(entry.0, *id);
            if header.infix_symbol.is_some() {
                assert_eq!(registry.signature(&entry.1), Some(header));
            }
        }
        for symbol in ["+", "-", "*", "missing"] {
            for lhs in [Ty::i64(), Ty::u8(), Ty::Unit] {
                for rhs in [Ty::i64(), Ty::u8(), Ty::Unit] {
                    assert_eq!(
                        registry.resolve(symbol, &lhs, &rhs),
                        eager.resolve(symbol, &lhs, &rhs)
                    );
                }
            }
        }
    }
}

#[test]
fn registry_can_be_reborrowed_for_a_shorter_lowering_scope() {
    fn shorten<'short, 'long: 'short>(
        registry: &'short OperatorRegistry<'long>,
    ) -> &'short OperatorRegistry<'short> {
        registry
    }
    let local = bindings();
    let shared = HashMap::new();
    let registry = OperatorRegistry::with_shared_signatures(&local, &shared);
    let short = shorten(&registry);
    assert_eq!(
        short.resolve("+", &Ty::i64(), &Ty::i64()),
        registry.resolve("+", &Ty::i64(), &Ty::i64())
    );
}
