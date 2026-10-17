mod common;
use common::{compile_exe, compile_exe_checked};
use ginc::cli::Profile;
use ginc::compile::{CompileFailure, CompileResult};
use std::process::Command;
use test_fixtures::{TempPackage, host_target_triple};

#[test]
fn partially_consumed_records_cannot_be_returned_for_a_double_commit() {
    let package = TempPackage::new("double_commit");
    package.write_flask("double_commit");
    let input = package.write(
        "main.gin",
        r#"Byte is in 0 to 255
Byte.finish(self): return
Transaction has
    state Byte
    commit(self): return
Pair has
    first Transaction
    second Transaction
bad(pair Pair) Pair:
    pair.first.commit
    return pair
#entry
main:
    pair: bad(Pair(first: Transaction(state: 1), second: Transaction(state: 2)))
    pair.first.commit
    pair.second.commit
    return 0
"#,
    );
    for profile in [Profile::Debug, Profile::Release] {
        for emit in ["obj", "exe", "mlir", "interface"] {
            let output = package.join(format!("rejected-{profile:?}-{emit}"));
            let result = Command::new(env!("CARGO_BIN_EXE_ginc"))
                .arg(&input)
                .args(["--target", &host_target_triple()])
                .args(["--profile", profile.as_str()])
                .args(["--emit", emit])
                .arg("--no-cache")
                .arg("--output")
                .arg(&output)
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                stderr.contains("[type-use-of-maybe-moved-value]"),
                "{profile:?}, {emit:?}: {stderr}"
            );
            assert!(!result.status.success(), "{profile:?}, {emit:?}: {stderr}");
            assert!(stderr.contains("type checking failed"), "{stderr}");
            assert!(result.stdout.is_empty(), "{result:?}");
            assert!(!output.exists(), "{profile:?}, {emit:?}");
        }
    }
}

#[test]
fn ownership_flaws_stop_object_emission() {
    for (code, body) in [
        (
            "type-use-of-moved-value",
            concat!(
                "#entry\nmain:\n    tx: Transaction(state: 1)\n",
                "    tx.commit\n    tx.commit\n    return 0\n",
            ),
        ),
        (
            "type-owned-param-not-consumed",
            concat!(
                "inspect(value ref Byte): return\nforget(value own Byte):\n    inspect(value)\n",
                "#entry\nmain:\n    forget(1 as Byte)\n    return 0\n",
            ),
        ),
        (
            "type-rebind-would-discard-value",
            concat!(
                "#entry\nmain:\n    tx: Transaction(state: 1)\n",
                "    tx:: Transaction(state: 2)\n    tx.commit\n    return 0\n",
            ),
        ),
        (
            "type-overlapping-target-group-arguments",
            concat!(
                "write(value mut{a} Byte, source ref{b} Byte): return value:: deref source\n",
                "#entry\nmain:\n    value: 1 as Byte\n    write(value, value)\n    return 0\n",
            ),
        ),
    ] {
        let package = TempPackage::new("ownership_flaws");
        package.write_flask("ownership_flaws");
        let input = package.write(
            "main.gin",
            &format!(
                concat!(
                    "Byte is in 0 to 255\nByte.finish(self): return\nTransaction has\n",
                    "    state Byte\n    commit(self): return\n{body}",
                ),
                body = body,
            ),
        );
        let artifact = package.join("rejected.o");
        let result = Command::new(env!("CARGO_BIN_EXE_ginc"))
            .arg(input)
            .args(["--emit", "obj", "--target", &host_target_triple()])
            .arg("--no-cache")
            .arg("--output")
            .arg(&artifact)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.contains(&format!("[{code}]")), "{stderr}");
        assert!(!result.status.success(), "{stderr}");
        assert!(stderr.contains("type checking failed"), "{stderr}");
        assert!(!artifact.exists(), "{code}");
    }
}

#[test]
fn imported_terminal_methods_finish_transactions_without_a_release_trait() {
    let package = TempPackage::new("terminal_transactions");
    package.write_flask("terminal_transactions");
    package.write(
        "database/transaction.gin",
        r#"Word is in 0 to 255
Word has
    replace(previous Self, next Self) Self: return next
    copy(self ref) Self: return deref self
Word.release(self):
    return
Transaction has
    completed @Word
    commit(self, result Word) Word:
        pointer: self.completed
        pointer.[0]:: Word.replace(pointer.[0], result.copy)
        return result
    rollback(self):
        pointer: self.completed
        pointer.[0]:: Word.replace(pointer.[0], 2)
        return
"#,
    );
    let input = package.write(
        "main.gin",
        r#"use terminal_transactions.database Transaction, Word
Wrapper has
    tx Transaction
    cancel(self):
        Wrapper(tx) := self
        tx.rollback
        return
#entry
main:
    first Word: 0
    second Word: 0
    tx: Transaction(completed: @first)
    result: tx.commit(1)
    other: Wrapper(tx: Transaction(completed: @second))
    other.cancel
    first_value is in Word: first
    second_value is in Word: second
    result_value is in Word: result
    return when first_value = 1 and second_value = 2 and result_value = 1 then 0 else 1
"#,
    );
    for profile in [Profile::Debug, Profile::Release] {
        let output = package.join(format!("program-{profile:?}"));
        compile_exe_checked(input.clone(), &output, profile);
        assert!(
            Command::new(output).status().unwrap().success(),
            "{profile:?}"
        );
    }
    package.write("main.gin", "use terminal_transactions.database Transaction, Word\nforget(tx Transaction):\n    return\n#entry\nmain:\n    completed Word: 0\n    tx: Transaction(completed: @completed)\n    forget(tx)\n    return 0\n");
    let output = package.join("rejected");
    let result = compile_exe(input, &output.clone(), Profile::Debug);
    assert!(
        matches!(
            result,
            CompileResult::Failed(CompileFailure::TypecheckFailed)
        ),
        "{result:?}"
    );
    assert!(!output.exists());
}

#[test]
fn missing_terminal_methods_stop_compilation_even_for_unused_scalars() {
    for declaration in [
        "Unused is in 0 to 1\n",
        "Unused is in 0 to 1\nfinish(value Unused):\n    return\n",
        "Unused is in 0 to 1\nUnused.observe(self ref):\n    return\n",
    ] {
        let package = TempPackage::new("missing_terminal_exit");
        package.write_flask("missing_terminal_exit");
        let input = package.write("main.gin", &format!("{declaration}#entry\nmain: 0\n"));
        let output = package.join("rejected");
        let result = compile_exe(input, &output.clone(), Profile::Debug);
        assert!(
            matches!(
                result,
                CompileResult::Failed(CompileFailure::TypecheckFailed)
            ),
            "{declaration}: {result:?}"
        );
        assert!(!output.exists());
    }
}

#[test]
fn unused_records_and_unions_need_no_terminal_methods() {
    for declaration in [
        "Unused has value [in 0 to 1; 1]\n",
        "Unused has value [in 0 to 1; 1]\nfinish(value Unused):\n    return\n",
        "Unused has\n    value [in 0 to 1; 1]\n    observe(self ref):\n        return\n",
        "Payload has value [in 0 to 1; 1]\nUnused is Present(Payload) or Empty\n",
    ] {
        let package = TempPackage::new("unused_wrapper");
        package.write_flask("unused_wrapper");
        let input = package.write("main.gin", &format!("{declaration}#entry\nmain: 0\n"));
        let output = package.join("program");
        let result = compile_exe(input, &output.clone(), Profile::Debug);
        assert!(result.is_success(), "{declaration}: {result:?}");
        assert!(!result.has_typecheck_diagnostics(), "{result:?}");
        assert!(Command::new(output).status().unwrap().success());
    }
}

#[test]
fn terminal_methods_cannot_discard_same_module_transaction_fields() {
    let package = TempPackage::new("unhandled_transaction");
    package.write_flask("unhandled_transaction");
    let input = package.write(
        "main.gin",
        r#"Word is in 0 to 255
Word.finish(self):
    return
Transaction has
    state Word
    commit(self):
        return
    rollback(self):
        return
Wrapper has
    tx Transaction
    release(self):
        return
#entry
main: 0
"#,
    );
    let output = package.join("rejected");
    let result = compile_exe(input, &output.clone(), Profile::Debug);
    assert!(
        matches!(
            result,
            CompileResult::Failed(CompileFailure::TypecheckFailed)
        ),
        "{result:?}"
    );
    assert!(!output.exists());
}
