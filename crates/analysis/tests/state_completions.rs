use analysis::PackageCache;
use test_fixtures::TempPackage;

const MODEL: &str = "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\nCount is in 0 to 255\nCount has Numeric\n    release(self):\n        return\nRegion has\n    #private\n    initialized Count\n    initialized_count(self ref) is in Count: return self.initialized.as_num\n    take_last(self mut and self.initialized_count > 0) Count: return 0\n    #private\n    hidden(self ref) Count: return 0\n    release(self):\n        return\nmake(value Count) Region: return Region(value)\n";

#[test]
fn member_completions_use_branch_facts_across_modules_and_hide_private_members() {
    let package = TempPackage::new("state_completions");
    package.write_flask("app");
    package.write("model/region.gin", MODEL);
    let body = "use app.model Region\nread(region mut Region):\n    when region.initialized_count > 0 then\n        return\n    region.|\n    return\n";
    let position = body.find('|').unwrap();
    let source = body.replace('|', "");
    let path = package.write("main.gin", &source).canonicalize().unwrap();
    let cache = PackageCache::for_test_with_file(path.clone());
    let labels: Vec<_> = cache
        .completions_at(&path, position as u32)
        .into_iter()
        .map(|candidate| candidate.label)
        .collect();
    assert!(labels.contains(&"release".to_owned()), "{labels:?}");
    assert!(!labels.contains(&"take_last".to_owned()), "{labels:?}");
    assert!(!labels.contains(&"initialized".to_owned()), "{labels:?}");
    assert!(!labels.contains(&"hidden".to_owned()), "{labels:?}");
    let source = source.replace(
        "when region.initialized_count > 0",
        "when region.initialized_count = 0",
    );
    cache.set_contents(&path, source);
    let labels: Vec<_> = cache
        .completions_at(&path, position as u32)
        .into_iter()
        .map(|candidate| candidate.label)
        .collect();
    assert!(labels.contains(&"take_last".to_owned()), "{labels:?}");
}

#[test]
fn completion_uses_unsaved_preconditions_from_an_imported_module() {
    let package = TempPackage::new("state_completion_dependency_edit");
    package.write_flask("app");
    let model = package
        .write("model/region.gin", MODEL)
        .canonicalize()
        .unwrap();
    let body = "use app.model Region\nread(region mut Region and region.initialized_count = 1):\n    region.|\n    return\n";
    let position = body.find('|').unwrap();
    let path = package
        .write("main.gin", &body.replace('|', ""))
        .canonicalize()
        .unwrap();
    let cache = PackageCache::for_test_with_file(model.clone());
    cache.add_file(path.clone()).unwrap();
    let visible = || {
        cache
            .completions_at(&path, position as u32)
            .iter()
            .any(|candidate| candidate.label == "take_last")
    };
    assert!(visible());
    cache.set_contents(
        &model,
        MODEL.replace("self.initialized_count > 0", "self.initialized_count > 1"),
    );
    assert!(!visible());
}
