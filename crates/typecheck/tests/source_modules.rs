use typecheck::{CheckedSourceModule, Limits, SourceCheckError, SourceType, check_module};

const WINDOW: &str = include_str!("../examples/window-source.gin");
const CLIENT: &str = include_str!("../examples/window-client.gin");

fn compile(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("valid module syntax");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(source, before);
    result
}

fn accepts(name: &str, text: &str, dependencies: &[&CheckedSourceModule]) -> CheckedSourceModule {
    compile(name, text, dependencies).unwrap_or_else(|e| panic!("{e:?}\n{text}"))
}

fn rejects(name: &str, text: &str, dependencies: &[&CheckedSourceModule], expected: &str) {
    match compile(name, text, dependencies) {
        Ok(_) => panic!("invalid module was checked:\n{text}"),
        Err(error) => assert_eq!(format!("{error:?}"), expected, "{text}"),
    }
}

fn library() -> CheckedSourceModule {
    accepts("demo.window", WINDOW, &[])
}

#[test]
fn a_separate_client_uses_only_the_checked_library_artifact() {
    let library = library();
    assert_eq!(library.name(), "demo.window");
    assert_eq!(library.checked().functions.len(), 3);
    let client = accepts("app.client", CLIENT, &[&library]);
    let function = client
        .checked()
        .functions
        .iter()
        .find(|f| f.name == "roundtrip")
        .unwrap();
    assert_eq!(function.type_parameters, ["item"]);
    assert_eq!(function.result, SourceType::Generic { index: 0 });
}

#[test]
fn renaming_the_entire_library_and_client_preserves_capabilities() {
    let renamed_library = WINDOW
        .replace("Window", "Parcel")
        .replace("window", "parcel")
        .replace("payload", "cargo")
        .replace("lower", "start")
        .replace("upper", "finish");
    let renamed_client = CLIENT
        .replace("demo.window", "unrelated.package")
        .replace("Window", "Parcel")
        .replace("window", "parcel");
    let library = accepts("unrelated.package", &renamed_library, &[]);
    accepts("different.client", &renamed_client, &[&library]);
    let forge = "use unrelated.package Parcel
forge(item Type, value item) Parcel(item):
    return Parcel(cargo: value, start: 0, finish: 0)
";
    rejects("different.client", forge, &[&library], "Privacy");
}

#[test]
fn private_fields_cannot_be_read_constructed_or_destructured_by_clients() {
    let library = library();
    for text in [
        "use demo.window Window
forge(item Type, value item) Window(item):
    return Window(payload: value, lower: 0, upper: 0)
",
        "use demo.window Window
peek(item Type, window Window(item)) item:
    return window.payload
",
        "use demo.window Window
peek(item Type, window Window(item)) item:
    return (window).payload
",
        "use demo.window Window, make_window
peek(item Type, value item) item:
    return make_window(item, value, 0, 0).payload
",
        "use demo.window Window, make_window
peek(item Type, value item) item:
    return (make_window(item, value, 0, 0)).payload
",
        "use demo.window Window
unpack(item Type, window Window(item)) item:
    Window(value, lo, hi) := window
    return value
",
    ] {
        rejects("app.client", text, &[&library], "Privacy");
    }
}

#[test]
fn call_sites_prove_relational_requirements_instead_of_assuming_them() {
    let library = library();
    let text = "use demo.window Window, make_window
bad(item Type, value item, lo in 0 to 16, hi in 0 to 16) Window(item):
    return make_window(item, value, lo, hi)
";
    rejects("app.client", text, &[&library], "Unproved");
    let invalid = "use demo.window Window, make_window
bad(item Type, value item) Window(item):
    return make_window(item, value, 5, 0)
";
    rejects("app.client", invalid, &[&library], "Unproved");
    let out_of_range = invalid.replace("5, 0", "-1, 0");
    rejects("app.client", &out_of_range, &[&library], "Type");
}

#[test]
fn calls_transfer_inputs_and_returned_children_without_duplication_or_drop() {
    let library = library();
    for text in [
        "use demo.window Window, make_window, take_window
bad(item Type, value item, lo in 0 to 16, hi in 0 to 16 and >= lo) item:
    first: make_window(item, value, lo, hi)
    second: make_window(item, value, lo, hi)
    return take_window(item, second)
",
        "use demo.window Window, take_window
bad(item Type, window Window(item)) item:
    moved: take_window(item, window)
    return take_window(item, window)
",
    ] {
        rejects("app.client", text, &[&library], "UseAfterMove");
    }
    for text in [
        "use demo.window make_window
bad(item Type, value item):
    make_window(item, value, 0, 0)
    return
",
        "use demo.window Window, take_window
bad(item Type, window Window(item)):
    take_window(item, window)
    return
",
    ] {
        rejects("app.client", text, &[&library], "Unconsumed");
    }
}

#[test]
fn exported_numeric_refinements_are_established_by_real_library_bodies() {
    let library = accepts(
        "demo.numeric",
        "echo(value in 0 to 16) in 0 to 16 and >= value: return value\n",
        &[],
    );
    let client = "use demo.numeric echo
forward(value in 0 to 16) in 0 to 16 and >= value:
    return echo(value)
";
    accepts("app.numeric", client, &[&library]);
    rejects(
        "demo.numeric",
        "echo(value in 0 to 16) in 0 to 16 and >= value: return 0\n",
        &[],
        "Unproved",
    );
    rejects(
        "app.numeric",
        &client.replace("echo(value)", "0"),
        &[&library],
        "Unproved",
    );
}

#[test]
fn body_changes_revalidate_inferred_return_identity() {
    let identity = accepts(
        "demo.identity",
        "identity(value in 0 to 16) in 0 to 16: return value\n",
        &[],
    );
    let replacement = accepts(
        "demo.identity",
        "identity(value in 0 to 16) in 0 to 16: return 0\n",
        &[],
    );
    let client = "use demo.identity identity
same(value in 0 to 16) in 0 to 16 and = value: return identity(value)
";
    accepts("app.client", client, &[&identity]);
    rejects("app.client", client, &[&replacement], "Unproved");
}

#[test]
fn a_signature_or_an_invalid_owned_body_cannot_publish_an_interface() {
    rejects(
        "demo.incomplete",
        "identity(item Type, value item) item\n",
        &[],
        "MissingBody",
    );
    rejects(
        "demo.invalid",
        "forget(item Type, value item): return\n",
        &[],
        "Unconsumed",
    );
}

#[test]
fn client_names_do_not_capture_library_nominal_types_or_metatype_roles() {
    let library = library();
    let spoof = "use demo.window make_window
Window(item) has payload item, lower in 0 to 16, upper in 0 to 16 and >= lower
spoof(item Type, value item) Window(item): return make_window(item, value, 0, 0)
";
    rejects("app.client", spoof, &[&library], "Type");
    let metatype = "use demo.window Window, make_window
Type has
Marker has
make(value Marker) Window(Marker): return make_window(Marker, value, 0, 0)
";
    accepts("app.client", metatype, &[&library]);
}

#[test]
fn imported_type_named_type_is_not_a_metatype_parameter() {
    let owned = accepts("types", "Token has\nType has payload Token\n", &[]);
    rejects(
        "app.client",
        "use types Type\nforget(value Type): return\n",
        &[&owned],
        "Unconsumed",
    );
    let generic = accepts("generic_types", "Type(item) has value item\n", &[]);
    rejects(
        "app.client",
        "use generic_types Type\nforget(value Type): return\n",
        &[&generic],
        "Type",
    );
}

#[test]
fn manual_parameter_roles_cannot_panic_during_publication() {
    let original = "f(item Type, value item) item: return value\n";
    let mut source = parser::parse_strict(original).unwrap();
    source.text = original.replace("item", "Type").into();
    let ast::Item::Binding(binding) = &mut source.items[0].value else {
        unreachable!();
    };
    let parameters = &mut binding.parameters.as_mut().unwrap().value;
    parameters[0].name.value = "Type".into();
    for annotation in [
        parameters[1].annotation.as_mut().unwrap(),
        binding.annotation.as_mut().unwrap(),
    ] {
        let ast::Annotation::Expression(expression) = &mut annotation.value else {
            unreachable!();
        };
        expression.value = ast::Expr::Name("Type".into());
    }
    let before = source.clone();
    let outcome =
        std::panic::catch_unwind(|| check_module("manual", &source, &[], Limits::default()));
    assert!(
        outcome.is_ok(),
        "inconsistent role resolution must not index out of bounds"
    );
    if let Ok(module) = outcome.unwrap() {
        assert_eq!(module.checked().functions[0].parameters.len(), 1);
        assert_eq!(module.checked().functions[0].type_parameters, ["Type"]);
    }
    assert_eq!(source, before);
}

#[test]
fn different_modules_do_not_share_nominal_identity_by_spelling() {
    let first = library();
    let other = WINDOW
        .replace("make_window", "make_other")
        .replace("rebuild_window", "rebuild_other")
        .replace("take_window", "take_other");
    let second = accepts("demo.other", &other, &[]);
    let client = "use demo.window Window
use demo.other make_other
wrong(item Type, value item) Window(item): return make_other(item, value, 0, 0)
";
    rejects("app.client", client, &[&first, &second], "Type");
}

#[test]
fn duplicate_dependency_names_cannot_grant_defining_module_privileges() {
    let library = library();
    rejects("demo.window", CLIENT, &[&library], "Duplicate");
    rejects("app.client", CLIENT, &[&library, &library], "Duplicate");
}

#[test]
fn a_local_value_is_not_an_imported_callable() {
    let library = library();
    let client = "use demo.window Window, take_window
bad(item Type, window Window(item)) item:
    take_window: 0
    return take_window(item, window)
";
    assert!(compile("app.client", client, &[&library]).is_err());
}

#[test]
fn imports_require_available_verified_exports() {
    let library = library();
    assert!(compile("app.client", CLIENT, &[]).is_err());
    assert!(compile("app.client", "use demo.window nonexistent\n", &[&library]).is_err());
}

#[test]
fn utf8_and_span_id_collisions_between_modules_do_not_change_checking() {
    let library = accepts("demo.window", &format!("-- hé 🦀\n{WINDOW}"), &[]);
    accepts("app.client", &format!("-- naïve 🦀\n{CLIENT}"), &[&library]);
}

#[test]
fn forward_calls_compose_argument_identity_and_cycles_are_unsupported() {
    let implementation = "first(x in 0 to 16, y in 0 to 16) in 0 to 16 and >= x:
    return second(y, x)
second(a in 0 to 16, b in 0 to 16) in 0 to 16:
    return b
";
    let library = accepts("demo.forward", implementation, &[]);
    let client = "use demo.forward first
client(x in 0 to 16, y in 0 to 16) in 0 to 16 and >= x:
    return first(x, y)
";
    accepts("app.client", client, &[&library]);
    rejects(
        "demo.cyclic",
        "cycle(value in 0 to 16) in 0 to 16: return cycle(value)\n",
        &[],
        "Unsupported",
    );
}

#[test]
fn function_type_arguments_and_runtime_arity_are_checked_separately() {
    let library = library();
    for arguments in ["item, value, 0", "item, value, 0, 0, 0"] {
        let source = format!(
            "use demo.window Window, make_window
bad(item Type, value item) Window(item): return make_window({arguments})
"
        );
        rejects("app.client", &source, &[&library], "Type");
    }
    let mismatch = "use demo.window Window, make_window
bad(a Type, b Type, value b) Window(a): return make_window(a, value, 0, 0)
";
    rejects("app.client", mismatch, &[&library], "Type");
}

#[test]
fn client_work_depends_on_the_interface_not_library_body_length() {
    let short = accepts(
        "demo.identity",
        "identity(value in 0 to 16) in 0 to 16: return value\n",
        &[],
    );
    let mut implementation = String::from("identity(value in 0 to 16) in 0 to 16:\n");
    for index in 0..40 {
        implementation.push_str(&format!("    unused{index}: 0\n"));
    }
    implementation.push_str("    return value\n");
    let long = accepts("demo.identity", &implementation, &[]);
    let text = "use demo.identity identity
forward(value in 0 to 16) in 0 to 16 and = value: return identity(value)
";
    let source = parser::parse_strict(text).unwrap();
    let before = source.clone();
    let mut first_success = None;
    for work_max in 0..=2048 {
        let limits = Limits {
            work_max,
            ..Limits::default()
        };
        let first = check_module("app.client", &source, &[&short], limits);
        let second = check_module("app.client", &source, &[&long], limits);
        match (first, second) {
            (Ok(_), Ok(_)) => {
                first_success.get_or_insert(work_max);
            }
            (Err(left), Err(right)) => {
                assert_eq!(left, SourceCheckError::Budget);
                assert_eq!(right, left);
            }
            _ => panic!("client budget depends on implementation at {work_max}"),
        }
    }
    assert!(first_success.is_some());
    assert_eq!(source, before);
}

#[test]
fn unused_body_only_instances_do_not_leak_into_client_work() {
    let prefix = "Marker has\nBox(item) has value item\n\
identity(value in 0 to 16) in 0 to 16:\n";
    let short = accepts("demo.identity", &format!("{prefix}    return value\n"), &[]);
    let implementation = format!(
        "{prefix}    temp: Box(value: Marker())
    Box(marker) := temp
    Marker() := marker
    return value
"
    );
    let long = accepts("demo.identity", &implementation, &[]);
    assert_eq!(short.checked().instances, long.checked().instances);
    let source = parser::parse_strict(
        "use demo.identity identity\n\
forward(value in 0 to 16) in 0 to 16 and = value: return identity(value)\n",
    )
    .unwrap();
    let mut observed_success = false;
    for work_max in 0..=2048 {
        let limits = Limits {
            work_max,
            ..Limits::default()
        };
        let short_result = check_module("app.client", &source, &[&short], limits);
        let long_result = check_module("app.client", &source, &[&long], limits);
        match (short_result, long_result) {
            (Ok(_), Ok(_)) => observed_success = true,
            (Err(left), Err(right)) => {
                assert_eq!(left, SourceCheckError::Budget);
                assert_eq!(right, left);
            }
            _ => panic!("body-only type instances affect client budget at {work_max}"),
        }
    }
    assert!(observed_success);
}

#[test]
fn a_proof_claim_cannot_end_an_owned_parameter_by_calling_a_function() {
    let library = "Marker has
consume(marker Marker, value in 0 to 0) in 0 to 0:
    Marker() := marker
    return value
";
    accepts("demo.pure", library, &[]);
    for body in [
        "bad(marker Marker) out in 0 to 0 where out = consume(marker, 0):
    return 0
",
        "bad(value in 0 to 0 and >= consume(Marker(), 0)) in 0 to 0: return value\n",
        "bad(value in 0 to 0) in 0 to 0 and >= consume(Marker(), 0): return value\n",
        "bad(value in 0 to 0) out in 0 to 0 where out = consume(Marker(), 0): return value\n",
    ] {
        rejects("demo.pure", &format!("{library}{body}"), &[], "Unsupported");
    }
}
