//! Unused-import checks must use flat bound names (`Architecture`), not export paths.

use ast::Import;
use parser::query::SourceParseExt;

fn body_without_use_lines(source: &str) -> String {
    source
        .lines()
        .filter(|l| !l.trim_start().starts_with("use "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn unused_bundle_member_names(import: &Import, body: &str) -> Vec<String> {
    let mut unused = Vec::new();
    for mi in &import.0 {
        for (name, _) in mi.bound_name_spans() {
            let name = name.to_string();
            if name.len() >= 2 && !body.contains(name.as_str()) {
                unused.push(name);
            }
        }
    }
    unused
}

#[test]
fn dotted_bundle_members_not_unused_when_flat_name_used_in_body() {
    let source = "\
use core.(default.Default, target.arch.Architecture, target.os.OperatingSystem, target.vendor.Vendor)

Target has arch Architecture, vendor Vendor, os OperatingSystem
Target.Default has default: (arch: 'x86_64', vendor: 'unknown', os: 'unknown')
";
    let output = source.parse_source_full();
    let import = &output.ast.uses[0];
    let body = body_without_use_lines(source);
    let unused = unused_bundle_member_names(import, &body);
    assert!(
        unused.is_empty(),
        "expected no unused bundle imports, reported unused: {unused:?}"
    );
}

#[test]
fn bound_name_spans_use_flat_names() {
    let source = "use core.(target.arch.Architecture)\n";
    let output = source.parse_source_full();
    let mi = &output.ast.uses[0].0[0];
    let names: Vec<_> = mi
        .bound_name_spans()
        .into_iter()
        .map(|(n, _)| n.to_string())
        .collect();
    assert_eq!(names, vec!["Architecture"]);
}
