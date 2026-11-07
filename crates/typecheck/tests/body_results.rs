//! D9: which value a recorded body returns. The evaluator takes the block's `result`; the
//! first `return` among its statements must be the same node, for every body.
use typecheck::{CheckedSourceBodyKind as Kind, CheckedSourceFile, Limits, check_file};

fn assert_one_result(file: &CheckedSourceFile, label: &str) -> usize {
    let mut blocks = 0;
    for (index, body) in file.bodies.iter().enumerate() {
        let Some(body) = body else { continue };
        let Kind::Block { statements, result } = &body.expressions[body.root as usize].kind else { continue };
        blocks += 1;
        let returns: Vec<u32> = statements
            .iter()
            .filter_map(|s| match body.expressions[*s as usize].kind {
                Kind::Return { value } => Some(value),
                _ => None,
            })
            .collect();
        let Kind::Return { value } = body.expressions[*result as usize].kind else {
            panic!("{label}: `{}` result is not a return", file.functions[index].name);
        };
        assert_eq!(returns.first(), Some(&value), "{label}: `{}`", file.functions[index].name);
        assert_eq!(returns.len(), 1, "{label}: `{}` has several returns", file.functions[index].name);
    }
    blocks
}

#[test]
fn the_result_of_every_recorded_body_is_its_first_and_only_return() {
    let mut blocks = 0;
    let mut files = 0;
    let programs = [
        "clamp(n in 0 to 9) in 0 to 7:\n    when n < 2 then return 0\n    when n > 8 then return 7\n    return n - 2\n",
        "first(n in 0 to 9) in 0 to 20:\n    i in 0 to 10: 0\n    while i < 10 do\n        when i * i > n then return i\n        i :: i + 1\n    return 20\n",
        "grade(n in 0 to 9) in 0 to 3:\n    return when\n        n < 3 then 0\n        n < 8 then 2\n        else 3\n",
        "total(n in 0 to 9) in 0 to 99:\n  s in 0 to 99: 0\n  for i in 1 to n do\n    s +: i\n  return s\n",
        "#unrestricted\nMode is Quiet or Loud\npick(n in 0 to 9) Mode:\n    when n < 3 then return Mode.Quiet\n    return Mode.Loud\nlevel(n in 0 to 9) in 0 to 9:\n    when pick(n) is Loud then return 5\n    return 1\n",
        "id(element, x element) element: return x\n",
    ];
    for program in programs {
        let file = check_file(&parser::parse_strict(program).unwrap(), Limits::default()).unwrap();
        blocks += assert_one_result(&file, program);
        files += 1;
    }
    // And every example the checker accepts on its own.
    for entry in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/examples")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "gin") {
            let text = std::fs::read_to_string(&path).unwrap();
            if let Ok(parsed) = parser::parse_strict(&text) {
                if let Ok(file) = check_file(&parsed, Limits::default()) {
                    blocks += assert_one_result(&file, &path.display().to_string());
                    files += 1;
                }
            }
        }
    }
    // Negative control: the check looked at something.
    assert!(files > 6 && blocks > 10, "{files} files, {blocks} blocks");
}
