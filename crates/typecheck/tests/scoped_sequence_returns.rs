mod support;

use support::check;

const PRELUDE: &str = "Byte is in 0 to 255\nByte has finish(self): return\nView has\n    length Byte\n    data ref [Byte; length]\n    finish(self): return\n";

#[test]
fn promoted_arrays_survive_return_routes() {
    for body in [
        "ref [1 as Byte]",
        "\n    data: ref [1 as Byte]\n    return data",
        "\n    when flag is 0 then\n        return ref [1 as Byte]\n    return ref [2 as Byte]",
        "\n    while flag is 0 do\n        when flag is 0 then\n            return ref [1 as Byte]\n    return ref [2 as Byte]",
    ] {
        let source = format!(
            "{PRELUDE}make(flag Byte) ref [Byte; 1]: {return_prefix}{body}\n",
            return_prefix = if body.starts_with('\n') {
                ""
            } else {
                "return "
            }
        );
        let typed = check(&source);
        assert!(
            typed.all_flaws().is_empty(),
            "{source}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn ordinary_records_preserve_static_and_parameter_borrows() {
    for body in [
        "make(input Byte) View:\n    data: ref [1 as Byte]\n    return View(length: 1 as Byte, data: data)",
        "make(input ref [Byte; 1]) View: return View(length: 1 as Byte, data: input)",
        "Container has\n    view View\n    finish(self): return self.view.finish\nmake(input ref [Byte; 1]) Container:\n    view: View(length: 1 as Byte, data: input)\n    return Container(view: view)",
    ] {
        let typed = check(&format!("{PRELUDE}{body}\n"));
        assert!(
            typed.all_flaws().is_empty(),
            "{body}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn local_array_borrows_cannot_escape_through_records_or_branches() {
    for body in [
        "make(input Byte) View:\n    source [Byte; 1]: [input]\n    return View(length: 1 as Byte, data: source)",
        "make(flag Byte) ref [Byte; 1]:\n    source [Byte; 1]: [1]\n    when flag is 0 then\n        return ref source\n    return ref [2 as Byte]",
        "Container has\n    view View\n    finish(self): return self.view.finish\nmake(input Byte) Container:\n    source [Byte; 1]: [input]\n    view: View(length: 1 as Byte, data: source)\n    return Container(view: view)",
    ] {
        let typed = check(&format!("{PRELUDE}{body}\n"));
        assert!(
            typed
                .all_flaws()
                .iter()
                .any(|(_, flaw)| flaw.code.slug().starts_with("type-reference-return-")),
            "{body}: {:?}",
            typed.all_flaws()
        );
    }
}

#[test]
fn promoted_storage_cannot_be_upgraded_to_mutable() {
    let typed = check(&format!(
        "{PRELUDE}write(data mut [Byte; 1]): return\nmake(input Byte):\n    data: ref [1 as Byte]\n    write(data)\n    return\n"
    ));
    assert!(!typed.all_flaws().is_empty());
}
