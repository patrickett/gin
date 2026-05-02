use parser::parse_source_full;
use typeck::TyEnv;

#[test]
fn typo_near_import_shows_did_you_mean() {
    let source = "use core\n\nmain:\n    cor\nreturn\n";
    let out = parse_source_full(source);
    let env = TyEnv::from_file_ast(&out.ast);
    let mut symptoms = Vec::new();
    env.check_unknowns(&out.ast, &mut symptoms);
    let d = symptoms
        .iter()
        .find(|d| d.message.contains("undefined binding"))
        .expect("expected undefined binding diagnostic");
    assert!(
        !d.message.contains("did you mean"),
        "primary message should be the error only: {}",
        d.message
    );
    let hint = d.help.as_ref().expect("expected hint text");
    assert!(hint.contains("did you mean"), "hint: {hint}");
    assert!(hint.contains("`core`"), "hint: {hint}");
}
