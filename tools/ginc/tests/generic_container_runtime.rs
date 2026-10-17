mod common;
use common::compile_and_run_exe;
use ginc::cli::Profile;
use test_fixtures::TempPackage;

#[test]
fn imported_generic_methods_use_concrete_element_operations_and_layouts() {
    for profile in [Profile::Debug, Profile::Release] {
        let package = TempPackage::new("generic_container");
        package.write_flask("generic_container");
        package.write(
            "holder/holder.gin",
            r#"Holder(element) has
    finish(self):
        return
    value element

    new(element, value element) Self: return Self(value: value)

    copy_value(self ref) element: return element.copy(self.value)

    width(element) Size: return #size(element) as Size

    release(self own):
        Holder(value: value) := self
        element.release(value)
        Holder.finish(element)
        return

    finish(element):
        return

Size is in 0 to 255
Size has
    finish(self):
        return
"#,
        );
        let input = package.write(
            "main.gin",
            r#"use generic_container.holder as boxes

Small is in 0 to 255
Small has
    finish(self):
        return
    copy(self ref) Self: return deref self

    release(self own):
        Small.discard(self)
        return

    discard(value Self):
        return

Wide is in 0 to 65535
Wide has
    finish(self):
        return
    copy(self ref) Self: return deref self

    release(self own):
        Wide.discard(self)
        return

    discard(value Self):
        return

#entry
main:
    small: boxes.Holder.new(Small, 7 as Small)
    wide: boxes.Holder.new(value: 513 as Wide, element: Wide)
    first is in Small: boxes.Holder.copy_value(small)
    second is in Wide: boxes.Holder.copy_value(wide)
    size is in 0 to 255: boxes.Holder.width(Wide)
    boxes.Holder.release(small)
    boxes.Holder.release(wide)
    return when first = 7 and second = 513 and size = 2 then 0 else 1
"#,
        );
        let executable = package.join("program");
        let output = compile_and_run_exe(input, &executable, profile, "", "run generic methods");
        assert_eq!(output.status.code(), Some(0), "{profile:?}");
    }
}
