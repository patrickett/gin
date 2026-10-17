use analysis::PackageCache;
use test_fixtures::TempPackage;

#[test]
fn allocation_hover_hides_private_fields_and_preserves_direct_method_hover() {
    let pkg = TempPackage::new("allocation_type_hover");
    pkg.write_flask("sample");
    pkg.write("alloc/count.gin", "Count(x) is in 0 to 1024\n");
    let source = include_str!("../../../modules/core/alloc/allocator.gin");
    let path = pkg.write("alloc/allocator.gin", source);
    let use_source = "use sample.alloc Allocation\nByte is in 0 to 255\nblock Allocation(Byte)\n";
    let use_path = pkg.write("main.gin", use_source);
    let engine = PackageCache::for_test_with_file(path.clone());
    engine.add_file(use_path.clone()).unwrap();

    let expected = "```gin\nAllocation(x)\n```";
    let byte = source.find("Allocation(x)").unwrap();
    let hover = engine.hover(&path, byte as u32).unwrap();
    assert!(hover.markdown.contains(expected), "{}", hover.markdown);
    assert!(!hover.markdown.contains("reclaim"), "{}", hover.markdown);
    assert_eq!(hover.byte_range, Some(byte..byte + "Allocation".len()));

    for (byte, _) in use_source.match_indices("Allocation") {
        let hover = engine.hover(&use_path, byte as u32).unwrap();
        assert!(hover.markdown.contains(expected), "{}", hover.markdown);
        assert!(!hover.markdown.contains("reclaim"), "{}", hover.markdown);
        assert_eq!(hover.byte_range, Some(byte..byte + "Allocation".len()));
    }

    let byte = source.find("reclaim(self,").unwrap();
    let hover = engine.hover(&path, byte as u32).unwrap();
    assert!(
        hover.markdown.contains("reclaim(self,"),
        "{}",
        hover.markdown
    );
}

#[test]
fn fieldless_generic_type_hover_shows_name_and_parameters() {
    let pkg = TempPackage::new("fieldless_type_hover");
    pkg.write_flask("sample");
    let source =
        "Marker(x) has\n    new() Self: return Self()\n    release(self):\n        return\n";
    let path = pkg.write("marker.gin", source);
    let engine = PackageCache::for_test_with_file(path.clone());

    let hover = engine.hover(&path, 0).unwrap();
    assert_eq!(
        hover.markdown,
        "```gin\nsample\n```\n\n```gin\nMarker(x)\n```"
    );
}

#[test]
fn public_type_hover_excludes_private_members_and_compiler_queries() {
    let pkg = TempPackage::new("private_type_hover");
    pkg.write_flask("sample");
    let source = "#bits(8)\nByte is in 0 to 255\nRecord has\n    #private\n    secret Byte\n    visible Byte\n    #private\n    hidden(self ref) Byte: return self.secret\n    #inline\n    read(self ref) Byte: return self.visible\n    release(self):\n        return\n";
    let path = pkg.write("model/record.gin", source);
    let use_source = "use sample.model Record\nvalue Record\n";
    let use_path = pkg.write("main.gin", use_source);
    let engine = PackageCache::for_test_with_file(path.clone());
    engine.add_file(use_path.clone()).unwrap();
    for (path, byte) in [
        (&path, source.find("Record has").unwrap()),
        (&use_path, use_source.rfind("Record").unwrap()),
    ] {
        let hover = engine.hover(path, byte as u32).unwrap();
        assert!(
            hover.markdown.contains("Record has visible Byte"),
            "{}",
            hover.markdown
        );
        for absent in ["secret", "hidden", "#private", "#inline"] {
            assert!(!hover.markdown.contains(absent), "{}", hover.markdown);
        }
    }
    let byte = source.find("Byte is").unwrap();
    let hover = engine.hover(&path, byte as u32).unwrap();
    assert!(!hover.markdown.contains("#bits"), "{}", hover.markdown);
    let byte = source.find("read(self").unwrap();
    let hover = engine.hover(&path, byte as u32).unwrap();
    assert!(
        hover.markdown.contains("read(self ref) Byte"),
        "{}",
        hover.markdown
    );
    assert!(!hover.markdown.contains("#inline"), "{}", hover.markdown);
}

#[test]
fn changing_field_visibility_refreshes_imported_type_hover() {
    let pkg = TempPackage::new("private_hover_update");
    pkg.write_flask("sample");
    let source = "Byte is in 0 to 255\nRecord has\n    #private\n    secret Byte\n    visible Byte\n    release(self):\n        return\n";
    let path = pkg.write("model/record.gin", source);
    let use_source = "use sample.model Record\nvalue Record\n";
    let use_path = pkg.write("main.gin", use_source);
    let engine = PackageCache::for_test_with_file(path.clone());
    engine.add_file(use_path.clone()).unwrap();
    let byte = use_source.rfind("Record").unwrap() as u32;
    let hover = engine.hover(&use_path, byte).unwrap();
    assert!(!hover.markdown.contains("secret"), "{}", hover.markdown);
    engine.set_contents(&path, source.replace("    #private\n", ""));
    let hover = engine.hover(&use_path, byte).unwrap();
    assert!(hover.markdown.contains("secret Byte"), "{}", hover.markdown);
    engine.set_contents(&path, source.to_string());
    let hover = engine.hover(&use_path, byte).unwrap();
    assert!(!hover.markdown.contains("secret"), "{}", hover.markdown);
}

#[test]
fn inaccessible_members_do_not_provide_hover_signatures() {
    let pkg = TempPackage::new("private_member_hover");
    pkg.write_flask("sample");
    let source = "Byte is in 0 to 255\nRecord has\n    #private\n    secret Byte\n    #private\n    hidden(self ref) Byte: return self.secret\n    visible(self ref) Byte: return self.secret\n    release(self):\n        return\n";
    let path = pkg.write("model/record.gin", source);
    let use_source = "use sample.model Record, Byte\nread_secret(value ref Record) Byte: return value.secret\nread_hidden(value ref Record) Byte: return value.hidden\nread_visible(value ref Record) Byte: return value.visible\n";
    let use_path = pkg.write("main.gin", use_source);
    let engine = PackageCache::for_test_with_file(path.clone());
    engine.add_file(use_path.clone()).unwrap();
    for member in ["secret", "hidden"] {
        let byte = use_source.find(&format!("value.{member}")).unwrap() + "value.".len();
        let hover = engine.hover(&use_path, byte as u32);
        assert!(hover.is_none(), "{member}: {hover:?}");
    }
    let byte = use_source.find("value.visible").unwrap() + "value.".len();
    let hover = engine.hover(&use_path, byte as u32).unwrap();
    assert!(
        hover.markdown.contains("visible(self ref) Byte"),
        "{}",
        hover.markdown
    );
}

#[test]
fn changing_method_visibility_refreshes_its_hover() {
    let pkg = TempPackage::new("private_method_update");
    pkg.write_flask("sample");
    let source = "Byte is in 0 to 255\nByte has\n    release(self):\n        return\nRecord has\n    #private\n    read(self ref) Byte: return 0 as Byte\n    release(self):\n        return\n";
    let path = pkg.write("model/record.gin", source);
    let use_source =
        "use sample.model Record, Byte\nread(value ref Record) Byte: return value.read\n";
    let use_path = pkg.write("main.gin", use_source);
    let engine = PackageCache::for_test_with_file(path.clone());
    engine.add_file(use_path.clone()).unwrap();
    let byte = (use_source.find("value.read").unwrap() + "value.".len()) as u32;
    assert!(engine.hover(&use_path, byte).is_none());
    engine.set_contents(&path, source.replace("    #private\n", ""));
    let hover = engine.hover(&use_path, byte).unwrap();
    assert!(
        hover.markdown.contains("read(self ref) Byte"),
        "{}",
        hover.markdown
    );
    engine.set_contents(&path, source.to_string());
    assert!(engine.hover(&use_path, byte).is_none());
}
