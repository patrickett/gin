mod support;

use support::package;

const MODEL: &str = r#"Byte is in 0 to 255
Byte has
    release(self):
        return
Record has
    #private
    secret Byte
    visible Byte
    new(secret own Byte, visible own Byte) Self: return Self(secret, visible)
    read(self ref) Byte: return self.secret
    #private
    hidden(self ref) Byte: return self.secret
    #private
    make(secret own Byte, visible own Byte) Self: return Self(secret, visible)
    release(self):
        return
"#;

const BLOCK_MODEL: &str = r#"Byte is in 0 to 255
Byte has
    release(self):
        return
Record has
    #private
        secret Byte
        hidden(self ref) Byte:
            return self.secret
        make(secret own Byte, visible own Byte) Self: return Self(secret, visible)
    visible Byte
    new(secret own Byte, visible own Byte) Self: return Self(secret, visible)
    read(self ref) Byte: return self.hidden
    release(self):
        return
"#;

fn expect(model: &str, client: &str, expected: &[&str]) {
    let outputs = package("sample", &[("model", model), ("client", client)]);
    let flaws: Vec<_> = outputs.iter().flat_map(|file| file.all_flaws()).collect();
    let mut codes: Vec<_> = flaws.iter().map(|(_, flaw)| flaw.code.slug()).collect();
    codes.sort();
    codes.dedup();
    let mut expected = expected.to_vec();
    expected.sort();
    expected.dedup();
    assert_eq!(codes, expected, "{flaws:?}\n{client}");
}

#[test]
fn public_fields_and_methods_remain_accessible() {
    expect(
        MODEL,
        "use sample.model Record, Byte\nread(value ref Record) Byte: return value.visible\ncall(value ref Record) Byte: return value.read\ncreate(a own Byte, b own Byte) Record: return Record.new(a, b)\n",
        &[],
    );
}

#[test]
fn private_blocks_protect_fields_and_both_kinds_of_methods() {
    for operation in [
        "read(value ref Record) Byte: return value.secret\n",
        "read(value ref Record) Byte: return value.hidden\n",
        "create(a own Byte, b own Byte) Record: return Record.make(a, b)\n",
        "create(a own Byte, b own Byte) Record: return Record(secret: a, visible: b)\n",
    ] {
        let client = format!("use sample.model Record, Byte\n{operation}");
        expect(BLOCK_MODEL, &client, &["type-private-member"]);
        let granted = BLOCK_MODEL.replace("#private", "#private(client)");
        expect(&granted, &client, &[]);
        let ungranted = BLOCK_MODEL.replace("#private", "#private(other)");
        expect(&ungranted, &client, &["type-private-member"]);
    }
}

#[test]
fn private_block_grants_preserve_terminal_authority_for_destructuring() {
    let client = "use sample.model Record, Byte\nunpack(value Record) Byte:\n    Record(secret, visible) := value\n    visible.release\n    return secret\n";
    expect(BLOCK_MODEL, client, &["type-private-member"]);
    let granted = BLOCK_MODEL.replace("#private", "#private(client)");
    expect(&granted, client, &["type-terminal-authority"]);
}

#[test]
fn private_block_permissions_end_on_dedent_and_allow_the_defining_module() {
    expect(
        BLOCK_MODEL,
        "use sample.model Record, Byte\nread(value ref Record) Byte: return value.visible\ncall(value ref Record) Byte: return value.read\ncreate(a own Byte, b own Byte) Record: return Record.new(a, b)\n",
        &[],
    );
    let model = format!("{BLOCK_MODEL}\ninside(value ref Record) Byte: return value.hidden\n");
    expect(&model, "", &[]);
}

#[test]
fn private_block_grants_validate_and_accept_multiple_modules() {
    for grants in ["client, other", "other, client"] {
        let model = BLOCK_MODEL.replace("#private", &format!("#private({grants})"));
        expect(
            &model,
            "use sample.model Record, Byte\nread(value ref Record) Byte: return value.hidden\n",
            &[],
        );
    }
    for grants in ["42", "client, 42", ""] {
        let model = BLOCK_MODEL.replace("#private", &format!("#private({grants})"));
        expect(&model, "", &["type-invalid-private-grant"]);
    }
}

#[test]
fn private_fields_cannot_be_read_or_written() {
    expect(
        MODEL,
        "use sample.model Record, Byte\nread(value ref Record) Byte: return value.secret\n",
        &["type-private-member"],
    );
    expect(
        MODEL,
        "use sample.model Record, Byte\nreplace(previous Byte, next own Byte) Byte: return next\nwrite(value mut Record, next own Byte): return value.secret:: replace(value.secret, next)\n",
        &["type-private-member"],
    );
}

#[test]
fn private_fields_prevent_direct_construction_and_destructuring() {
    expect(
        MODEL,
        "use sample.model Record, Byte\ncreate(a own Byte, b own Byte) Record: return Record(secret: a, visible: b)\n",
        &["type-private-member"],
    );
    expect(
        MODEL,
        "use sample.model Record, Byte\nunpack(value Record) Byte:\n    Record(secret, visible) := value\n    visible.release\n    return secret\n",
        &["type-private-member"],
    );
}

#[test]
fn private_instance_and_associated_methods_cannot_be_called() {
    expect(
        MODEL,
        "use sample.model Record, Byte\nread(value ref Record) Byte: return value.hidden\n",
        &["type-private-member"],
    );
    expect(
        MODEL,
        "use sample.model Record, Byte\ncreate(a own Byte, b own Byte) Record: return Record.make(a, b)\n",
        &["type-private-member"],
    );
}

#[test]
fn private_members_are_accessible_in_the_defining_module() {
    let model = format!("{MODEL}\ninside(value ref Record) Byte: return value.hidden\n");
    expect(&model, "", &[]);
}

#[test]
fn explicit_module_grants_apply_to_individual_members() {
    let model = MODEL.replacen("#private", "#private(client)", 1);
    expect(
        &model,
        "use sample.model Record, Byte\nread(value ref Record) Byte: return value.secret\n",
        &[],
    );
    expect(
        &model,
        "use sample.model Record, Byte\nread(value ref Record) Byte: return value.hidden\n",
        &["type-private-member"],
    );
}

#[test]
fn private_is_rejected_on_a_type() {
    expect(
        "#private\nThing has\n    release(self):\n        return\n",
        "",
        &["type-private-member-context"],
    );
}

#[test]
fn private_grants_require_module_names() {
    expect(
        &MODEL.replacen("#private", "#private(42)", 1),
        "",
        &["type-invalid-private-grant"],
    );
}

#[test]
fn private_overloads_do_not_hide_public_overloads() {
    let model = format!(
        "{MODEL}\nRecord has\n    choose(first own Byte) Byte: return first\n    #private\n    choose(first own Byte, second own Byte) Byte:\n        first.release\n        return second\n"
    );
    expect(
        &model,
        "use sample.model Record, Byte\nchoose(value own Byte) Byte: return Record.choose(value)\n",
        &[],
    );
    expect(
        &model,
        "use sample.model Record, Byte\nchoose(a own Byte, b own Byte) Byte: return Record.choose(a, b)\n",
        &["type-private-member"],
    );
}

#[test]
fn private_is_rejected_on_a_free_function() {
    expect(
        "#private\nwork:\n    return\n",
        "",
        &["type-private-member-context"],
    );
}

#[test]
fn generic_methods_keep_their_access_permissions() {
    let model = "Byte is in 0 to 255\nByte has\n    release(self):\n        return\nRecord(x) has\n    #private\n    hidden(self ref) Byte: return 0 as Byte\n    visible(self ref) Byte: return self.hidden\n    release(self):\n        return\n";
    expect(
        model,
        "use sample.model Record, Byte\nread(value ref Record(Byte)) Byte: return value.visible\n",
        &[],
    );
    expect(
        model,
        "use sample.model Record, Byte\nread(value ref Record(Byte)) Byte: return value.hidden\n",
        &["type-private-member"],
    );
}

#[test]
fn dot_type_respects_field_visibility() {
    let granted = MODEL.replacen("#private", "#private(client)", 1);
    for (model, field, allowed) in [
        (MODEL, "visible", true),
        (MODEL, "secret", false),
        (granted.as_str(), "secret", true),
    ] {
        let client = format!(
            "use sample.model Record, Byte\nread(value ref Record) Byte: return value.{field}\n"
        );
        let outputs = package("sample", &[("model", model), ("client", &client)]);
        let column = client.lines().nth(1).unwrap().find('.').unwrap() + 1;
        let query = outputs[1].dot_type(&client, 1, column as u32);
        assert_eq!(query.is_some(), allowed, "{field}: {query:?}");
    }
}

#[test]
fn dot_type_allows_private_fields_in_the_defining_module() {
    let model = format!("{MODEL}inside(value ref Record) Byte: return value.secret\n");
    let outputs = package("sample", &[("model", &model), ("client", "")]);
    let line = model.lines().count() - 1;
    let column = model.lines().nth(line).unwrap().find('.').unwrap() + 1;
    assert!(
        outputs[0]
            .dot_type(&model, line as u32, column as u32)
            .is_some()
    );
}

#[test]
#[should_panic(expected = "grouped parameter must have an ownership convention")]
fn grouped_arguments_reject_missing_convention_metadata() {
    use typecheck::typed::{ExprId, TypedCallableSignature};

    let client =
        "use sample.model Record, Byte\nread(value ref Record) Byte: return value.visible\n";
    let outputs = package("sample", &[("model", MODEL), ("client", client)]);
    let typed = &outputs[1];
    let bind = typed
        .defs
        .values()
        .find(|bind| {
            bind.name.as_str().ends_with("read")
                && bind
                    .params
                    .first()
                    .is_some_and(|(name, _)| name.as_str() == "value")
        })
        .unwrap();
    let mut signature = TypedCallableSignature::from(bind);
    assert_eq!(signature.params.len(), 1);
    assert!(signature.param_groups[0].is_some());
    let argument = (0..typed.exprs.kind.len())
        .map(|index| ExprId(index as u32))
        .find(|id| {
            typed.exprs.target_group_of(*id).is_some_and(|targets| {
                targets
                    .as_ref()
                    .is_some_and(|targets| targets.iter().next().is_some())
            })
        })
        .unwrap();
    signature.param_conventions.clear();
    typed.apply_target_groups(&[argument], &signature);
}

#[test]
#[should_panic(expected = "typed parameter must have an ownership convention")]
fn parameter_hover_rejects_missing_convention_metadata() {
    let client =
        "use sample.model Record, Byte\nread(value ref Record) Byte: return value.visible\n";
    let mut outputs = package("sample", &[("model", MODEL), ("client", client)]);
    let typed = &mut outputs[1];
    let bind = typed
        .defs
        .values_mut()
        .find(|bind| {
            bind.name.as_str().ends_with("read")
                && bind
                    .params
                    .first()
                    .is_some_and(|(name, _)| name.as_str() == "value")
        })
        .unwrap();
    assert_eq!(bind.params.len(), 1);
    bind.param_conventions.clear();
    let column = client.lines().nth(1).unwrap().find("value").unwrap();
    typed.hover_at(client, 1, column as u32);
}
