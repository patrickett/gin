use super::*;
use ast::ty::PackageInstanceKey;

fn declaration(name: &str) -> TypeDeclarationSemantics {
    let name = Intern::from_ref(name);
    TypeDeclarationSemantics {
        key: DeclarationKey {
            member: 0,
            package: PackageInstanceKey::workspace("sharing", "0"),
            module: Vec::new(),
            declaration: name,
        },
        display_name: name,
        parameters: Vec::new(),
        definition: Ty::Unit,
        sequence_literal: None,
        private_members: HashMap::new(),
        private_fields: HashSet::new(),
        field_refinements: HashMap::new(),
        reference: None,
        integer: None,
        discriminant: None,
    }
}

fn registry() -> (TypeRegistry, TypeId) {
    let id = TypeId {
        file: 1,
        name: Intern::from_ref("Original"),
    };
    let mut registry = TypeRegistry::default();
    registry.insert(id, declaration("Original"));
    (registry, id)
}

#[test]
fn completed_clones_share_tables_and_declaration_updates_are_isolated() {
    let (original, id) = registry();
    let mut changed = original.clone();
    assert!(Arc::ptr_eq(&original.tables, &changed.tables));
    changed.insert(id, declaration("Replacement"));
    assert_eq!(
        original.declaration(id).unwrap().display_name.as_str(),
        "Original"
    );
    assert_eq!(
        changed.declaration(id).unwrap().display_name.as_str(),
        "Replacement"
    );
    assert!(!Arc::ptr_eq(&original.tables, &changed.tables));
}

#[test]
fn visibility_and_target_updates_are_isolated() {
    let (original, id) = registry();
    let mut changed = original.clone();
    let method = crate::typed::DefId::named(Intern::from_ref("Original.hidden"));
    changed.insert_private_method(
        method,
        PrivateMethod {
            origin: None,
            file: 1,
            grants: Vec::new(),
        },
    );
    assert!(original.private_method(method).is_none());
    assert_eq!(changed.private_method(method).unwrap().file, 1);
    assert!(!Arc::ptr_eq(&original.tables, &changed.tables));
    assert_eq!(original.declaration(id), changed.declaration(id));
    let isolated_tables = Arc::as_ptr(&changed.tables);
    changed.defer_integer_layout(id);
    assert!(original.tables.target_dependent_integers.is_empty());
    assert!(
        changed
            .tables
            .target_dependent_integers
            .contains(&original.declaration(id).unwrap().key)
    );
    assert_eq!(isolated_tables, Arc::as_ptr(&changed.tables));
    assert_eq!(original.declaration(id), changed.declaration(id));
}

#[test]
fn merging_preserves_replacement_and_stale_handle_behavior() {
    let (original, id) = registry();
    let old_key = original.declaration(id).unwrap().key.clone();
    let mut incoming = original.clone();
    let replacement = declaration("Replacement");
    let new_key = replacement.key.clone();
    incoming.insert(id, replacement.clone());
    assert_eq!(incoming.handle(&old_key), Some(id));
    let mut empty = TypeRegistry::default();
    empty.merge_from(&incoming);
    assert_eq!(empty.handle(&old_key), None);
    assert_eq!(empty.handle(&new_key), Some(id));
    assert_eq!(empty.declaration(id), Some(&replacement));
    let mut populated = original.clone();
    populated.merge_from(&incoming);
    assert_eq!(populated.handle(&old_key), Some(id));
    assert_eq!(populated.handle(&new_key), Some(id));
    assert_eq!(populated.declaration(id), Some(&replacement));
    assert_eq!(original.declaration(id).unwrap().key, old_key);
    assert_eq!(original.handle(&new_key), None);
}

#[test]
fn shared_readers_keep_completed_answers_during_independent_writes() {
    let (original, id) = registry();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let reader = original.clone();
            scope.spawn(move || {
                for _ in 0..64 {
                    let declaration = reader.declaration(id).unwrap();
                    assert_eq!(declaration.display_name.as_str(), "Original");
                    assert_eq!(reader.handle(&declaration.key), Some(id));
                }
            });
        }
        let mut writer = original.clone();
        writer.insert(id, declaration("Replacement"));
        assert_eq!(
            writer.declaration(id).unwrap().display_name.as_str(),
            "Replacement"
        );
    });
    assert_eq!(
        original.declaration(id).unwrap().display_name.as_str(),
        "Original"
    );
}
