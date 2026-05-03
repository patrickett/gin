use std::fs;
use std::path::PathBuf;

use ginc::cli::{Args, Emit, Profile};
use ginc::compile::GinCompiler;

fn unique_temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    // Good enough for tests: pid + nanos.
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    dir.push(format!("gin_import_cycle_{name}_{pid}_{nanos}"));
    dir
}

#[test]
fn import_cycle_is_a_fatal_flaw() {
    let dir = unique_temp_dir("basic");
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // main -> a -> b -> a
    fs::write(
        dir.join("main.gin"),
        "use './a.gin' as a\n\nmain:\n    return 0\n",
    )
    .unwrap();
    fs::write(dir.join("a.gin"), "use './b.gin' as b\nx: 1\n").unwrap();
    fs::write(dir.join("b.gin"), "use './a.gin' as a\nx: 2\n").unwrap();

    let exe_path = dir.join("out.exe");
    let mut args = Args {
        input: dir.join("main.gin"),
        emit: Emit::Exe,
        output: Some(exe_path.clone()),
        profile: Profile::Debug,
        ..Default::default()
    };

    GinCompiler::compile(&mut args);

    assert!(
        !exe_path.exists(),
        "expected compilation to fail and not produce an executable"
    );

    let _ = fs::remove_dir_all(&dir);
}
