use ginc::cli::{Args, Emit};
use ginc::compile::{CompileResult, GinCompiler};

#[test]
fn import_cycle_fails_without_producing_an_artifact() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("mod.flask"),
        "#settings\nmodule: \"app\"\nversion: 0.1.0\ntarget: \"library\"\n",
    )
    .unwrap();
    for (module, source) in [
        ("a", "use app.b two\none() in 0 to 1: return two()\n"),
        ("b", "use app.a one\ntwo() in 0 to 1: return one()\n"),
    ] {
        std::fs::create_dir(directory.path().join(module)).unwrap();
        std::fs::write(directory.path().join(module).join("main.gin"), source).unwrap();
    }
    let artifact = directory.path().join("out.o");
    let result = GinCompiler::compile(&mut Args {
        input: directory.path().to_path_buf(),
        emit: Emit::Obj,
        output: Some(artifact.clone()),
        ..Default::default()
    });
    assert!(matches!(result, CompileResult::Failed(_)), "{result:?}");
    assert!(!artifact.exists());
}
