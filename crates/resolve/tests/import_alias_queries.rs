use parser::query::SourceParseExt;
use resolve::{ImportTarget, resolve_import_at};

#[test]
fn alias_queries_resolve_the_export_at_both_the_import_and_its_use() {
    for (source, export, alias) in [
        (
            "use 'values' count as total\nmain: total\n",
            "count",
            "total",
        ),
        (
            "use app.values count as total\nmain: total\n",
            "count",
            "total",
        ),
        ("use Count as Total\nmain: Total\n", "Count", "Total"),
    ] {
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        for byte in [
            source.find(export).unwrap(),
            source.find(alias).unwrap(),
            source.rfind(alias).unwrap(),
        ] {
            let target = resolve_import_at(&parsed.ast, source, byte).unwrap();
            let symbol = match target {
                ImportTarget::LocalBundleSymbol { symbol, .. }
                | ImportTarget::DepBundleMember { symbol, .. }
                | ImportTarget::CurrentModuleSymbol { symbol } => symbol,
                other => panic!("{other:?}"),
            };
            assert_eq!(symbol, export, "{source} at {byte}");
        }
    }
}
