use parser::query::SourceParseExt;
use typecheck::transform::{TransformCtx, member_completions};

const MODEL: &str = "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\nCount is in 0 to 255\nCount has Numeric\n    release(self):\n        return\nreplace(previous Count, next Count) Count: return next\nRegion has\n    capacity Count\n    initialized Count and <= capacity\n    initialized_count(self ref) is in Count: return self.initialized.as_num\n    take_last(self mut and self.initialized_count > 0) Count: return 0\n    clear(self mut):\n        self.initialized :: replace(self.initialized, 0)\n        return thus self.initialized_count = 0\n    release(self):\n        return\n";

fn labels(body: &str) -> Vec<String> {
    let source = format!("{MODEL}{body}");
    let position = source.find('|').unwrap();
    let source = source.replace('|', "");
    let (probe, receiver, _) =
        typecheck::completions::member_completion_input(&source, position).unwrap();
    let parsed = probe.parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}\n{probe}", parsed.symptoms);
    let mut ctx = TransformCtx::new();
    ctx.ide_package = true;
    member_completions(&parsed.ast, typecheck::FileId(0), &ctx, receiver)
        .into_iter()
        .map(|candidate| candidate.label)
        .collect()
}

#[test]
fn empty_receivers_do_not_offer_take_last() {
    let items =
        labels("run:\n    region: Region(4, 0)\n    region.|\n    region.release\n    return\n");
    assert!(!items.contains(&"take_last".to_owned()), "{items:?}");
    assert!(items.contains(&"release".to_owned()), "{items:?}");
}

#[test]
fn nonempty_and_unknown_receivers_still_offer_take_last() {
    for body in [
        "run:\n    region: Region(4, 1)\n    region.|\n    region.release\n    return\n",
        "run(region mut Region):\n    region.|\n    return\n",
    ] {
        let items = labels(body);
        assert!(items.contains(&"take_last".to_owned()), "{items:?}");
    }
}

#[test]
fn cleanup_removes_take_last_from_completion() {
    let items = labels(
        "run:\n    region: Region(4, 1)\n    region.clear\n    region.|\n    region.release\n    return\n",
    );
    assert!(!items.contains(&"take_last".to_owned()), "{items:?}");
    assert!(items.contains(&"release".to_owned()), "{items:?}");
}

#[test]
fn guarded_and_rejected_paths_have_different_completions() {
    let body = "read(region mut Region):\n    when region.initialized_count > 0 then\n        region.|\n        return\n    return\n";
    assert!(labels(body).contains(&"take_last".to_owned()));
    let body = "read(region mut Region):\n    when region.initialized_count > 0 then\n        return\n    region.|\n    return\n";
    assert!(!labels(body).contains(&"take_last".to_owned()));
}
