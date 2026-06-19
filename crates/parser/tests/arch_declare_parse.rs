//! Top-level `name Type` (e.g. `arch Architecture`) should parse as an unassigned bind.

use ast::BindValue;
use internment::Intern;
use parser::query::SourceParseExt;

#[test]
fn parse_top_level_unassigned_declare_like_bind() {
    let src = "\
Architecture is 'x86_64' or 'arm64'

arch Architecture
";
    let ast = src.parse_source_full().ast;
    let arch = ast.defs.get(&Intern::from_ref("arch")).unwrap_or_else(|| {
        panic!(
            "expected top-level `arch` def, defs={:?}, top_exprs={}",
            ast.defs.keys().collect::<Vec<_>>(),
            ast.exprs.len()
        )
    });
    assert!(
        matches!(arch.value, BindValue::Unassigned),
        "expected Unassigned bind, got {:?}",
        arch.value
    );
    assert!(
        arch.return_tag.as_ref().is_some_and(
            |t| matches!(&t.value, ast::TypeExpr::Nominal(n, _) if n.as_str() == "Architecture")
        ),
        "return_tag should be Architecture"
    );
}

#[test]
fn parse_full_arch_gin_fixture() {
    let src = "\
--- Target CPU architectures for conditional compilation.
Architecture is 'x86_64'
             or 'arm64'
             or 'wasm32'

arch Architecture
";
    let ast = src.parse_source_full().ast;
    assert!(ast.defs.contains_key(&Intern::from_ref("arch")));
}
