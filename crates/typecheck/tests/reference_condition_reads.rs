mod support;

use internment::Intern;
use support::check_parsed;
use typecheck::{DefId, TypedFileAst};

const PRELUDE: &str = "Int is in 0 to 100\nInt has\n    finish(self):\n        return\n";

fn reads_input(typed: &TypedFileAst, name: &str) -> bool {
    typed.defs[&DefId::named(Intern::from_ref(name))]
        .references
        .reads
        .iter()
        .any(|read| read.parameter == 0 && read.fields.is_empty())
}

#[test]
fn while_condition_records_a_dereferenced_parameter_read() {
    let typed = check_parsed(&format!(
        "{PRELUDE}read(input ref Int) Int:\n    while deref input is 0 do\n        marker Int: 1\n    return 0\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(reads_input(&typed, "read"));
}

#[test]
fn while_condition_propagates_a_called_functions_parameter_reads() {
    let typed = check_parsed(&format!(
        "{PRELUDE}observe(input ref Int) Int: return deref input\nread(input ref Int) Int:\n    while observe(input) is 0 do\n        marker Int: 1\n    return 0\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(reads_input(&typed, "observe"));
    assert!(reads_input(&typed, "read"));
}

#[test]
fn if_condition_still_records_a_dereferenced_parameter_read() {
    let typed = check_parsed(&format!(
        "{PRELUDE}read(input ref Int) Int:\n    when deref input is 0 then\n        return 1\n    return 0\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    assert!(reads_input(&typed, "read"));
}
