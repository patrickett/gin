//! Pre-refactor characterization of schema staging, error precedence and quotas.
use ast::{Item, SourceFile, TypeBody, TypeMember};
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

const SCHEMAS: &str = "\
Slot(element) is Empty or Full(element)
Box(element) has value element
Marker has
Bounds has
    #private
    lower in 0 to 16
    -- Constraints resolve sibling names, groups, and signed constants.
    upper in 0 to 16 and >= (lower) and >= (-1)
Container(element) has
    metadata Bounds
    pair (Box(element), @Marker)
    slots [Slot(element); 2]
make(lower in 0 to 16, upper in 0 to 16 and >= lower) Bounds:
    return Bounds(lower: lower, upper: upper)
read_upper(bounds Bounds) in 0 to 16:
    Bounds(lower, upper) := bounds
    return upper
";

const CLIENT: &str = "\
use case.schema Bounds, make, read_upper
observe(lower in 0 to 16, upper in 0 to 16 and >= lower) in 0 to 16 and = upper:
    return read_upper(make(lower, upper))
";

const BAD_CONSTRAINT: &str = "Early has value in 0 to 16 and >= missing\n";

fn run(
    source: &SourceFile,
    dependencies: &[&CheckedSourceModule],
    limits: Limits,
) -> Result<CheckedSourceModule, SourceCheckError> {
    let before = source.clone();
    let result = check_module("case.client", source, dependencies, limits);
    assert_eq!(source, &before);
    result
}

fn parse(text: &str) -> SourceFile {
    parser::parse_strict(text).expect("strict schema fixture")
}

fn library() -> CheckedSourceModule {
    let source = parse(SCHEMAS);
    check_module("case.schema", &source, &[], Limits::default()).unwrap()
}

fn rejects(source: &SourceFile, expected: SourceCheckError) {
    assert_eq!(
        run(source, &[], Limits::default()).unwrap_err(),
        expected,
        "{}",
        source.text
    );
}

#[test]
fn all_schema_types_are_resolved_before_any_schema_constraints() {
    for (later, expected) in [
        ("Late has value Missing\n", SourceCheckError::Unsupported),
        (
            "Box(element) has value element\nLate has value Box()\n",
            SourceCheckError::Type,
        ),
        (
            "Late has value in 0 to 16, value in 0 to 16\n",
            SourceCheckError::Duplicate,
        ),
        (
            "Late has\n    method(a in 0 to 1, self): return 0\n",
            SourceCheckError::MethodReceiver,
        ),
    ] {
        let source = parse(&format!("{BAD_CONSTRAINT}{later}"));
        rejects(&source, expected);
    }
    rejects(&parse(BAD_CONSTRAINT), SourceCheckError::UnknownName);
}

#[test]
fn malformed_later_field_syntax_precedes_earlier_constraint_resolution() {
    let text = format!("{BAD_CONSTRAINT}Late has\n    #private\n    value in 0 to 16\n");
    let mut source = parse(&text);
    let Item::Type(declaration) = &mut source.items[1].value else {
        unreachable!()
    };
    let TypeBody::Has(members) = &mut declaration.body.value else {
        unreachable!()
    };
    let TypeMember::Binding(field) = &mut members[0].value else {
        unreachable!()
    };
    field.attributes.clear();
    rejects(&source, SourceCheckError::Malformed);
}

#[test]
fn layout_consistency_and_cycles_remain_after_constraint_compilation() {
    for declaration in [
        "Recursive has next Recursive\n",
        "Bad(element) has payload element, upper in 0 to 16 and >= payload\n",
    ] {
        rejects(&parse(declaration), SourceCheckError::Type);
        let source = parse(&format!("{declaration}{BAD_CONSTRAINT}"));
        rejects(&source, SourceCheckError::UnknownName);
    }
}

#[test]
fn same_schema_shape_validation_precedes_field_type_resolution() {
    let source = parse("Bad has first Missing, second\n");
    rejects(&source, SourceCheckError::Unsupported);

    let source =
        parse("Bad has\n    first Missing\n    #private\n    #private\n    second in 0 to 16\n");
    rejects(&source, SourceCheckError::Duplicate);
}

#[test]
fn resolved_layouts_retain_order_privacy_and_generic_structure() {
    let source = parse(SCHEMAS);
    let checked = run(&source, &[], Limits::default()).unwrap();
    let schemas = &checked.checked().aggregates;
    assert_eq!(
        schemas
            .iter()
            .map(|schema| schema.name.as_str())
            .collect::<Vec<_>>(),
        // The union's own variants (`Slot.Empty`, `Slot.Full`) are layouts of the union, after the
        // declared types; they are not types of the module.
        ["Slot", "Box", "Marker", "Bounds", "Container", "Slot.Empty", "Slot.Full"]
    );
    let bounds = &schemas[3];
    assert_eq!(bounds.fields[0].name, "lower");
    assert!(bounds.fields[0].private);
    assert_eq!(bounds.fields[1].name, "upper");
    assert!(!bounds.fields[1].private);
    assert_eq!(schemas[4].parameters, ["element"]);
    assert_eq!(schemas[4].fields.len(), 3);
    let again = run(&source, &[], Limits::default()).unwrap();
    assert_eq!(checked.checked(), again.checked());
}

#[test]
fn imported_layouts_keep_constraints_and_use_site_privacy() {
    let library = library();
    run(&parse(CLIENT), &[&library], Limits::default()).unwrap();
    let invalid = CLIENT.replace("make(lower, upper)", "make(upper, lower)");
    assert_eq!(
        run(&parse(&invalid), &[&library], Limits::default()).unwrap_err(),
        SourceCheckError::Unproved
    );
    let private = parse(
        "use case.schema Bounds\n\
         construct() Bounds: return Bounds(lower: 0, upper: 0)\n",
    );
    assert_eq!(
        run(&private, &[&library], Limits::default()).unwrap_err(),
        SourceCheckError::Privacy
    );
}

fn budget(flow: bool, value: u32) -> Limits {
    if flow {
        Limits {
            flow_work_max: value,
            ..Limits::default()
        }
    } else {
        Limits {
            work_max: value,
            ..Limits::default()
        }
    }
}

fn minimum(
    source: &SourceFile,
    dependencies: &[&CheckedSourceModule],
    flow: bool,
    expected: Option<SourceCheckError>,
) -> u32 {
    let mut low = 0;
    let mut high = if flow {
        Limits::default().flow_work_max
    } else {
        Limits::default().work_max
    };
    assert_eq!(
        run(source, dependencies, budget(flow, high)).err(),
        expected
    );
    for _ in 0..32 {
        if low == high {
            break;
        }
        let midpoint = low + (high - low) / 2;
        let error = run(source, dependencies, budget(flow, midpoint)).err();
        if error == Some(SourceCheckError::Budget) {
            low = midpoint + 1;
        } else {
            assert_eq!(error, expected);
            high = midpoint;
        }
    }
    assert_eq!(low, high);
    assert!(low > 0);
    low
}

#[test]
fn successful_schema_and_import_budget_thresholds_are_frozen() {
    let library = library();
    for (name, source, dependencies, expected) in [
        // 5386 -> 5392: a parameter's own claims may now name the parameter (one more name to
        // search in each claim term).
        // 5392 -> 5403: a claim term tries the constant evaluator first (`cap - 4`), and a
        // bound check reads a local type's traits from its declaration; 5403 -> 5408: each type name asks the alias list first.
        // 5408 -> 5471: a claim term is first tried as arithmetic over parameters and queries
        // (`claim_arithmetic.rs`) before it is read as a constant.
        // 5471 -> 5488: a schema claim term may name an accessor (`linear.rs`, G4): the
        // accessor list is searched, and an arithmetic claim is related to its operands.
        // 5488 -> 4895 (flow 19 -> 16), 2026-11-04: `Slot(element) is Empty or Full(element)` declares
        // its own variants, so the two record declarations `Empty has` and `Full(element) has
        // value element` that the fixture used to need are gone: two fewer schemas to resolve.
        // 4895 -> 4883, 2026-11-05: `fact_edges` finds scalars through an index, not two scans per fact.
        ("schemas", parse(SCHEMAS), &[][..], (4883, 16)),
        // 2262 -> 2313: an importer copies the library's accessors (G4) and reads a plain
        // accessor of an imported type as its field.
        // 2313 -> 2264, 2026-11-04: the imported library has two fewer schemas (see "schemas").
        // 2264 -> 2242, 2026-11-05: `fact_edges` index (see "schemas").
        ("client", parse(CLIENT), &[&library][..], (2242, 7)),
    ] {
        let work = minimum(&source, dependencies, false, None);
        let flow = minimum(&source, dependencies, true, None);
        assert_eq!((work, flow), expected, "{name}");
        for (is_flow, limit) in [(false, work), (true, flow)] {
            assert!(run(&source, dependencies, budget(is_flow, limit)).is_ok());
            assert_eq!(
                run(&source, dependencies, budget(is_flow, limit - 1)).unwrap_err(),
                SourceCheckError::Budget
            );
        }
    }
}

#[test]
fn schema_error_budget_thresholds_are_frozen() {
    for (name, text, expected, expected_work) in [
        (
            "constraint",
            BAD_CONSTRAINT.to_owned(),
            SourceCheckError::UnknownName,
            332,
        ),
        (
            "type_before_constraint",
            format!("{BAD_CONSTRAINT}Box(element) has value element\nLate has value Box()\n"),
            SourceCheckError::Type,
            506,
        ),
        (
            "duplicate_before_constraint",
            format!("{BAD_CONSTRAINT}Late has value in 0 to 16, value in 0 to 16\n"),
            SourceCheckError::Duplicate,
            380,
        ),
    ] {
        let source = parse(&text);
        let work = minimum(&source, &[], false, Some(expected));
        assert_eq!(work, expected_work, "{name}");
        assert_eq!(
            run(&source, &[], budget(false, work)).unwrap_err(),
            expected
        );
        assert_eq!(
            run(&source, &[], budget(false, work - 1)).unwrap_err(),
            SourceCheckError::Budget
        );
    }
}
