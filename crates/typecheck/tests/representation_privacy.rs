//! Direct source checking, not a legacy conversion or a native pointer execution.
use typecheck::{CheckedSourceModule, Limits, SourceCheckError, check_module};

const LIBRARY: &str = include_str!("../examples/representation-library.gin");
const CLIENT: &str = include_str!("../examples/representation-client.gin");

fn compile(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
) -> Result<CheckedSourceModule, SourceCheckError> {
    let source = parser::parse_strict(text).expect("strict source syntax");
    let before = source.clone();
    let result = check_module(name, &source, dependencies, Limits::default());
    assert_eq!(
        source, before,
        "checking must not change the source snapshot"
    );
    result
}

fn accepts(name: &str, text: &str, dependencies: &[&CheckedSourceModule]) -> CheckedSourceModule {
    compile(name, text, dependencies).unwrap_or_else(|error| panic!("{error:?}\n{text}"))
}

fn rejects(
    name: &str,
    text: &str,
    dependencies: &[&CheckedSourceModule],
    expected: SourceCheckError,
) {
    assert_eq!(
        compile(name, text, dependencies).unwrap_err(),
        expected,
        "{text}"
    );
}

fn library() -> CheckedSourceModule {
    accepts("demo.representation", LIBRARY, &[])
}

#[test]
fn defining_reconstruction_and_existing_owned_transfers_are_supported() {
    let library = library();
    let client = accepts("app.client", CLIENT, &[&library]);
    assert_eq!(library.checked().functions.len(), 5);
    assert_eq!(client.checked().functions.len(), 4);
}

#[test]
fn foreign_fully_public_reconstruction_is_supported() {
    let library = library();
    accepts(
        "app.public",
        "use demo.representation Public\n\
         read(address @Public) Public: return #unchecked(address as Public)\n",
        &[&library],
    );
}

#[test]
fn two_unrelated_private_abstractions_reject_direct_raw_reconstruction() {
    let library = library();
    for target in ["Region", "Account"] {
        let client = format!(
            "use demo.representation {target}\n\
             forge(address @{target}) {target}: return #unchecked(address as {target})\n"
        );
        rejects("app.forge", &client, &[&library], SourceCheckError::Privacy);
    }
}

#[test]
fn owning_the_outer_declaration_does_not_grant_foreign_inner_authority() {
    let library = library();
    for target in ["Region", "Account"] {
        for visibility in ["", "    #private\n"] {
            let client = format!(
                "use demo.representation {target}\n\
                 Outer has\n{visibility}    inner {target}\n\
                 forge(address @Outer) Outer: return #unchecked(address as Outer)\n"
            );
            rejects("app.outer", &client, &[&library], SourceCheckError::Privacy);
        }
    }
}

#[test]
fn a_local_private_field_does_not_short_circuit_foreign_nested_checks() {
    let library = library();
    rejects(
        "app.mixed",
        "use demo.representation Region
Outer has
    #private
    stamp in 0 to 16
    region Region
forge(address @Outer) Outer: return #unchecked(address as Outer)
",
        &[&library],
        SourceCheckError::Privacy,
    );
}

#[test]
fn private_construction_still_rejects_while_existing_values_can_be_wrapped() {
    let library = library();
    rejects(
        "app.construct",
        "use demo.representation Marker, Region\n\
         forge(address @Marker) Region: return Region(address: address)\n",
        &[&library],
        SourceCheckError::Privacy,
    );
    accepts("app.wrap", CLIENT, &[&library]);
}

#[test]
fn typed_pointer_fields_do_not_embed_the_pointed_to_private_representation() {
    let library = library();
    accepts(
        "app.address",
        "use demo.representation Region\n\
         Address has value @Region\n\
         read(address @Address) Address: return #unchecked(address as Address)\n",
        &[&library],
    );
}

#[test]
fn instantiated_generic_fields_cannot_hide_a_foreign_private_value() {
    let library = library();
    rejects(
        "app.generic",
        "use demo.representation Region\n\
         Box(element) has value element\n\
         forge(address @Box(Region)) Box(Region): return #unchecked(address as Box(Region))\n",
        &[&library],
        SourceCheckError::Privacy,
    );
}

#[test]
fn unresolved_generic_reconstruction_cannot_publish_a_laundering_summary() {
    rejects(
        "app.unknown",
        "reconstruct(element Type, address @element) element: return #unchecked(address as element)\n",
        &[],
        SourceCheckError::Privacy,
    );
}

#[test]
fn forwarding_a_pointer_through_ordinary_values_does_not_grant_authority() {
    let library = library();
    let forwarding = accepts(
        "demo.forwarding",
        "Box(element) has value element\n\
         identity(element Type, value element) element: return value\n",
        &[],
    );
    rejects(
        "app.forwarded",
        "use demo.representation Region
use demo.forwarding Box, identity
forge(address @Region) Region:
    boxed: Box(value: identity(@Region, address))
    Box(pointer) := boxed
    return #unchecked(pointer as Region)
",
        &[&library, &forwarding],
        SourceCheckError::Privacy,
    );
    accepts(
        "app.forwarded_public",
        "use demo.representation Public
use demo.forwarding identity
read(address @Public) Public:
    pointer: identity(@Public, address)
    return #unchecked(pointer as Public)
",
        &[&library, &forwarding],
    );
}

#[test]
fn an_unused_type_argument_is_not_an_embedded_private_representation() {
    let library = library();
    accepts(
        "app.phantom",
        "use demo.representation Region\n\
         Phantom(element) has stamp in 0 to 16\n\
         read(address @Phantom(Region)) Phantom(Region): return #unchecked(address as Phantom(Region))\n",
        &[&library],
    );
}

#[test]
fn positional_and_alternative_containment_preserve_private_boundaries() {
    let library = library();
    for field in [
        "(Region,)",
        "[Region; 2]",
        "[Region; 0]",
        "(Public, Region)",
    ] {
        let text = format!(
            "use demo.representation Region, Public\n\
             Envelope has value {field}\n\
             forge(address @Envelope) Envelope: return #unchecked(address as Envelope)\n"
        );
        rejects(
            "app.positions",
            &text,
            &[&library],
            SourceCheckError::Privacy,
        );
    }
    rejects(
        "app.sum",
        "use demo.representation Region, Public\n\
         Choice is Public(Public) or Region(Region)\n\
         forge(address @Choice) Choice: return #unchecked(address as Choice)\n",
        &[&library],
        SourceCheckError::Privacy,
    );
}

#[test]
fn already_owned_private_children_still_move_exactly_once() {
    let library = library();
    let declarations = "use demo.representation Region\n\
                        Outer has inner Region\n";
    let duplicate = format!(
        "{declarations}bad(region Region) (Outer, Outer):
    first: Outer(inner: region)
    return (first, Outer(inner: region))
"
    );
    rejects(
        "app.duplicate",
        &duplicate,
        &[&library],
        SourceCheckError::UseAfterMove,
    );
    let dropped = format!("{declarations}bad(region Region): return\n");
    rejects(
        "app.drop",
        &dropped,
        &[&library],
        SourceCheckError::Unconsumed,
    );
}

#[test]
fn raw_pointer_casts_do_not_claim_numeric_or_unrelated_nominal_conversions() {
    let library = library();
    rejects(
        "app.mismatch",
        "use demo.representation Public, Region\n\
         wrong(address @Public) Region: return #unchecked(address as Region)\n",
        &[&library],
        SourceCheckError::Type,
    );
}

#[test]
fn display_names_and_type_renaming_do_not_grant_authority() {
    let renamed = LIBRARY
        .replace("Region", "Parcel")
        .replace("Account", "Ledger")
        .replace("Marker", "Tag");
    let library = accepts("other.library", &renamed, &[]);
    rejects(
        "app.renamed",
        "use other.library Parcel\n\
         Outer has child Parcel\n\
         forge(address @Outer) Outer: return #unchecked(address as Outer)\n",
        &[&library],
        SourceCheckError::Privacy,
    );
    let client = CLIENT
        .replace("demo.representation", "other.library")
        .replace("Region", "Parcel")
        .replace("Marker", "Tag");
    accepts("app.normal", &client, &[&library]);
}

#[test]
fn private_boundaries_survive_source_drop_reexport_and_diamond_compaction() {
    let library = library();
    let left = accepts(
        "graph.left",
        "use demo.representation Region\n\
         Left has child Region\n\
         forward(region Region) Region: return region\n",
        &[&library],
    );
    let right = accepts(
        "graph.right",
        "use demo.representation Region\n\
         Right has child Region\n\
         forward(region Region) Region: return region\n",
        &[&library],
    );
    drop(library);
    accepts(
        "graph.valid",
        "use graph.left Left, forward
use graph.right Right
roundtrip(left Left) Right:
    Left(child) := left
    return Right(child: forward(child))
",
        &[&left, &right],
    );
    for (module, target) in [("graph.left", "Left"), ("graph.right", "Right")] {
        let text = format!(
            "use {module} {target}\n\
             forge(address @{target}) {target}: return #unchecked(address as {target})\n"
        );
        rejects(
            "graph.forge",
            &text,
            &[&left, &right],
            SourceCheckError::Privacy,
        );
    }
}

#[test]
fn manual_source_erasure_cannot_publish_a_public_replacement_layout() {
    let text = "Secret has
    #private
    key in 0 to 16
    stamp in 0 to 16
";
    for mutation in 0..6 {
        let mut source = parser::parse_strict(text).unwrap();
        let body_start = text.find("has").unwrap();
        let shortened_body = source
            .span_table
            .insert(ast::Span::new(body_start, body_start + 3));
        let shortened_item = source
            .span_table
            .insert(ast::Span::new(0, body_start + 3));
        let ast::Item::Type(declaration) = &mut source.items[0].value else {
            unreachable!()
        };
        let ast::TypeBody::Has(members) = &mut declaration.body.value else {
            unreachable!()
        };
        match mutation {
            0 => {
                let ast::TypeMember::Binding(field) = &mut members[0].value else {
                    unreachable!()
                };
                field.attributes.clear();
            }
            1 => {
                members.remove(0);
            }
            2 => members.clear(),
            3 => members.reverse(),
            4 | 5 => {
                members.clear();
                declaration.body.span_id = shortened_body;
            }
            _ => unreachable!(),
        }
        if mutation == 5 {
            source.items[0].span_id = shortened_item;
        }
        assert_eq!(
            check_module("lib", &source, &[], Limits::default()).unwrap_err(),
            SourceCheckError::Malformed,
            "mutation {mutation}"
        );
    }
}

fn widen_span(
    source: &mut ast::SourceFile,
    id: ast::SpanId,
    end: usize,
) -> ast::SpanId {
    let start = source.span_table.get(id).start();
    source.span_table.insert(ast::Span::new(start, end))
}

fn hide_last_field_in_annotation(source: &mut ast::SourceFile, depth: u8) {
    let item = source.items.last().unwrap();
    let ast::Item::Type(declaration) = &item.value else {
        unreachable!()
    };
    let ast::TypeBody::Has(members) = &declaration.body.value else {
        unreachable!()
    };
    let ast::TypeMember::Binding(field) = &members[0].value else {
        unreachable!()
    };
    let mut annotation = field.annotation.clone().unwrap();
    let end = source.span_table.get(declaration.body.span_id).end();
    let member = members[0].span_id;
    let member = widen_span(source, member, end);
    annotation.span_id = widen_span(source, annotation.span_id, end);
    match &mut annotation.value {
        ast::Annotation::Domain { domain, .. } => {
            if depth >= 1 {
                domain.span_id = widen_span(source, domain.span_id, end);
            }
            if depth >= 2 {
                domain.base.span_id = widen_span(source, domain.base.span_id, end);
            }
        }
        ast::Annotation::Expression(expression) => {
            if depth >= 1 {
                expression.span_id = widen_span(source, expression.span_id, end);
            }
            if depth >= 2 {
                let ast::Expr::Call { arguments, .. } = &mut expression.value else {
                    unreachable!()
                };
                arguments.span_id = widen_span(source, arguments.span_id, end);
            }
        }
        _ => unreachable!(),
    }
    let ast::Item::Type(declaration) = &mut source.items.last_mut().unwrap().value else {
        unreachable!()
    };
    let ast::TypeBody::Has(members) = &mut declaration.body.value else {
        unreachable!()
    };
    members.truncate(1);
    members[0].span_id = member;
    let ast::TypeMember::Binding(field) = &mut members[0].value else {
        unreachable!()
    };
    field.annotation = Some(annotation);
}

#[test]
fn widened_annotation_wrappers_cannot_swallow_a_private_field() {
    for text in [
        "Secret has
    stamp in 0 to 16
    #private
    key in 0 to 16
",
        "Marker has
Box(element) has value element
Secret has
    stamp Box(Marker)
    #private
    key in 0 to 16
",
    ] {
        for depth in 0..3 {
            let mut source = parser::parse_strict(text).unwrap();
            hide_last_field_in_annotation(&mut source, depth);
            assert_eq!(
                check_module("lib", &source, &[], Limits::default()).unwrap_err(),
                SourceCheckError::Malformed,
                "depth {depth}: {text}"
            );
        }
    }
}

#[test]
fn field_fidelity_preserves_comments_without_erasing_private_boundaries() {
    let library = accepts(
        "commented",
        "Secret has
    -- field prefix
    #private
    -- literal punctuation: #private, has, as
    key in 0 to 16
    -- between fields
    stamp in 0 to 16
    -- trailing comment
make(key in 0 to 16) Secret: return Secret(key: key, stamp: 0)
-- EOF comment without a newline: #private",
        &[],
    );
    rejects(
        "app.comment",
        "use commented Secret\nforge(pointer @Secret) Secret: return #unchecked(pointer as Secret)\n",
        &[&library],
        SourceCheckError::Privacy,
    );
}

#[test]
fn reconstruction_needs_the_acceptance_before_privacy_is_even_asked() {
    let library = library();
    rejects(
        "app.unaccepted",
        "use demo.representation Public\n\
         read(address @Public) Public: return address as Public\n",
        &[&library],
        SourceCheckError::MissingAcceptance,
    );
}
