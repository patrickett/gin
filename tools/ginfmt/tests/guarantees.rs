//! The four guarantees over the corpus, each with a negative control: a deliberately broken
//! printer that the same check must reject.

mod common;

use common::Input;
use ginfmt::verify::{check_comments, check_idempotent, check_semantics, comments};
use ginfmt::{Options, format, print_unverified};

/// Inputs that are valid but badly laid out, by hand.
const MESSY: &[&str] = &[
    "run: -- trailing on a header\n    x: 1 -- after a binding\n    return x -- last\n",
    "use  core.io   print_int,read_int\n\n\n\nrun:\n\tprint_int( 1 )   --tabs\n\treturn 0\n",
    "Box has\n  value element   -- field\n  count   Int\n  take(self) element: return self.value\n",
    "f(a Int,\n      b Int) Int:\n  -- why\n  return a+b -- sum\n",
];

/// The same source with a different layout: indentation doubled and trailing comments added
/// (`double`), or indentation as tabs (`tabs`). Kept only when it still parses.
fn scrambled(source: &str) -> Vec<String> {
    let double: String = source
        .lines()
        .map(|line| {
            let body = line.trim_start_matches(' ');
            let indent = line.len() - body.len();
            let code = !body.is_empty() && !body.starts_with("--") && !body.contains("--");
            format!("{}{}{}\n", " ".repeat(indent * 2), body, if code && !body.contains(['\'', '"']) { " -- t" } else { "" })
        })
        .collect();
    let tabs: String = source
        .lines()
        .map(|line| {
            let body = line.trim_start_matches(' ');
            format!("{}{}\n", "\t".repeat((line.len() - body.len()) / 4), " ".repeat((line.len() - body.len()) % 4) + body)
        })
        .collect();
    [double, tabs].into_iter().filter(|text| common::parses(text)).collect()
}

fn corpus() -> Vec<Input> {
    let mut inputs: Vec<Input> = common::files().into_iter().filter(|i| common::parses(&i.text)).collect();
    inputs.extend(common::parser_test_inputs());
    for (at, text) in MESSY.iter().enumerate() {
        assert!(common::parses(text), "MESSY[{at}] must be valid");
        inputs.push(Input { name: format!("messy#{at}"), text: (*text).to_owned() });
    }
    let derived: Vec<Input> = inputs
        .iter()
        .filter(|i| i.name.starts_with("modules/") || i.name.starts_with("messy"))
        .flat_map(|i| {
            scrambled(&i.text).into_iter().enumerate().map(move |(n, text)| Input { name: format!("{}~{n}", i.name), text })
        })
        .collect();
    inputs.extend(derived);
    inputs
}

/// The real printer, which the corpus is sure to parse.
fn fmt(source: &str) -> String {
    format(source).unwrap_or_else(|e| panic!("corpus input fails to format: {e}\n{source}"))
}

#[test]
fn every_module_file_parses_and_formats() {
    let modules = common::module_files();
    assert!(modules.len() >= 58, "modules/ holds {} .gin files", modules.len());
    let rejected: Vec<_> = modules.iter().filter(|i| !common::parses(&i.text)).map(|i| i.name.as_str()).collect();
    assert!(rejected.is_empty(), "the new parser rejects: {rejected:?}");
    for input in &modules {
        fmt(&input.text);
    }
}

#[test]
fn corpus_is_large_and_not_vacuous() {
    let inputs = corpus();
    assert!(inputs.len() > 600, "{} inputs", inputs.len());
    let kinds = |prefix: &str| inputs.iter().filter(|i| i.name.starts_with(prefix)).count();
    eprintln!("corpus: {} inputs ({} modules/, {} parser-test literals, {} derived layouts)", inputs.len(), kinds("modules/"), kinds("crates/"), inputs.iter().filter(|i| i.name.contains('~')).count());
    let total_comments: usize = inputs.iter().map(|i| comments(&i.text).len()).sum();
    let trailing: usize = inputs.iter().map(|i| comments(&i.text).iter().filter(|c| c.2).count()).sum();
    assert!(total_comments > 200, "{total_comments} comments");
    assert!(trailing > 50, "{trailing} trailing comments");
    let changed = inputs.iter().filter(|i| fmt(&i.text) != i.text).count();
    assert!(changed > 20, "{changed} inputs are changed by formatting");
}

// (a) idempotence

#[test]
fn a_formatting_twice_is_formatting_once() {
    for input in corpus() {
        check_idempotent(&fmt, &input.text).unwrap_or_else(|e| panic!("{}: {e}", input.name));
        // The unverified printer too, so the final verification is not what makes it hold.
        let raw = |s: &str| print_unverified(s, Options::default()).unwrap();
        check_idempotent(&raw, &input.text).unwrap_or_else(|e| panic!("{} (unverified): {e}", input.name));
    }
}

#[test]
fn a_control_a_printer_that_grows_its_output_is_not_idempotent() {
    let broken = |s: &str| format!("{}-- pass\n", fmt(s));
    let error = check_idempotent(&broken, "run:\n    return 0\n").unwrap_err();
    assert!(error.contains("second pass differs"), "{error}");
    // Every corpus input is caught, not only a convenient one.
    assert!(corpus().iter().all(|i| check_idempotent(&broken, &i.text).is_err()));
}

// (b) semantics

#[test]
fn b_formatted_source_has_the_same_ast_ignoring_spans_and_trivia() {
    for input in corpus() {
        check_semantics(&input.text, &fmt(&input.text)).unwrap_or_else(|e| panic!("{}: {e}", input.name));
    }
}

#[test]
fn b_control_a_printer_that_changes_a_token_changes_the_ast() {
    let source = "sum(a Int, b Int) Int: return a + b\n";
    let broken = fmt(source).replace('+', "-");
    let error = check_semantics(source, &broken).unwrap_err();
    assert!(error.contains("different tokens"), "{error}");
}

#[test]
fn b_control_a_printer_that_loses_a_block_changes_the_ast_alone() {
    // Same tokens, different tree: only the AST comparison can see it.
    let source = "pair:\n    first: 1\n    second: 2\n";
    let formatted = fmt(source);
    let broken = formatted.replace("\n    second: 2", "\nsecond: 2");
    assert_eq!(ginfmt::verify::token_stream(&formatted), ginfmt::verify::token_stream(&broken));
    let error = check_semantics(source, &broken).unwrap_err();
    assert!(error.contains("different AST"), "{error}");
}

#[test]
fn b_control_a_printer_that_breaks_the_syntax_is_rejected() {
    let source = "run:\n    return 0\n";
    let error = check_semantics(source, "run:\nreturn 0 +\n").unwrap_err();
    assert!(error.contains("does not parse") || error.contains("different"), "{error}");
}

// (c) comments

#[test]
fn c_comments_are_neither_lost_nor_reordered_nor_moved_among_the_code() {
    for input in corpus() {
        check_comments(&input.text, &fmt(&input.text)).unwrap_or_else(|e| panic!("{}: {e}", input.name));
    }
}

const COMMENTED: &str = "\
-- first
use core.io print_int

run: -- trailing
    -- inside
    print_int(1) -- after call
    return 0
";

#[test]
fn c_control_dropping_a_comment_is_caught() {
    let broken = fmt(COMMENTED).replace("    -- inside\n", "");
    assert!(check_comments(COMMENTED, &broken).unwrap_err().contains("comments became"));
}

#[test]
fn c_control_reordering_two_comments_is_caught() {
    let formatted = fmt(COMMENTED);
    let broken = formatted.replace("-- first", "-- @@").replace("-- inside", "-- first").replace("-- @@", "-- inside");
    assert!(check_comments(COMMENTED, &broken).unwrap_err().contains("changed"));
}

#[test]
fn c_control_moving_a_comment_past_code_is_caught() {
    let broken = fmt(COMMENTED).replace("print_int(1) -- after call\n", "print_int(1)\n-- after call\n");
    assert!(check_comments(COMMENTED, &broken).is_err());
    let broken = fmt(COMMENTED).replace("run: -- trailing\n", "-- trailing\nrun:\n");
    assert!(check_comments(COMMENTED, &broken).is_err());
}

#[test]
fn comments_survive_with_their_text_and_their_place() {
    assert_eq!(fmt(COMMENTED), COMMENTED);
}

// wrapping

#[test]
fn wrapping_is_never_given_up_on_the_corpus() {
    for input in corpus() {
        let (_, gave_up) = ginfmt::format_detailed(&input.text, Options::default()).unwrap();
        assert!(!gave_up, "{}: the wrapped text broke a guarantee", input.name);
    }
}

// wrapping at 100 columns

const WIDTH: usize = 100;

fn names(count: usize, stem: &str) -> Vec<String> {
    (0..count).map(|n| format!("{stem}_{n}_with_a_longish_name")).collect()
}

/// Valid sources with lines that must be broken to fit.
fn long_variants() -> Vec<(&'static str, String)> {
    let args = names(9, "argument").join(", ");
    let params = names(8, "parameter").iter().map(|n| format!("{n} Int")).collect::<Vec<_>>().join(", ");
    let tags = |stem: &str| (0..9).map(|n| format!("{stem}{n}WithALongishName")).collect::<Vec<_>>();
    let traits = tags("Trait").join(" and ");
    let variants = tags("Variant").join(" or ");
    let chain = names(7, "condition_flag").iter().map(|n| format!("{n} = 1")).collect::<Vec<_>>().join(" and ");
    vec![
        ("call", format!("run:\n    result: compute({args})\n    return result\n")),
        ("nested call", format!("run:\n    return outer(inner({args}), other({args}))\n")),
        ("params", format!("compute({params}) Int:\n    return 0\n")),
        ("method params", format!("Thing has\n    value Int\n\n    compute(self, {params}) Int:\n        return 0\n")),
        ("trait list", format!("Thing has {traits}\n")),
        ("union", format!("Choice is {variants}\n")),
        ("conditional", format!("run:\n    result: when {} < 1 then {} else {}\n    return result\n", names(1, "left")[0], names(1, "right_value")[0].repeat(2), names(1, "other_value")[0].repeat(2))),
        ("and chain in args", format!("run:\n    return check(first_argument, {chain}, last_argument_name)\n")),
        ("array", format!("run:\n    items: [{args}]\n    return items\n")),
        ("operators in args", format!("run:\n    return total({})\n", names(8, "quantity").join(" + "))),
        ("trailing comment", format!("run:\n    result: compute({args}) -- stays on the last line\n    return result\n")),
        ("where claims", format!("clear(self mut) where {}:\n    return\n", (0..6).map(|n| format!("self.long_field_name_{n} :: self.long_field_name_{n} + 1")).collect::<Vec<_>>().join(", "))),
        ("header with params and claims", format!("fill(self mut, value item) piece Live(item) where {}:\n    return\n", (0..5).map(|n| format!("self.long_field_name_{n} :: self.long_field_name_{n} - 1")).collect::<Vec<_>>().join(", "))),
        ("method claims", format!("Thing has\n    length Int\n\n    clear(self mut) where {}:\n        return\n", (0..5).map(|n| format!("self.long_field_name_{n} :: self.long_field_name_{n} + 1")).collect::<Vec<_>>().join(", "))),
        ("use members", format!("use core.slice {}\n", (0..9).map(|n| format!("Member{n}WithALongishName")).collect::<Vec<_>>().join(", "))),
        ("variant claims", format!("f(self mut) Ok where ({}) or\n    Bad where (self :: self):\n    return\n", (0..6).map(|n| format!("self.long_field_name_{n} >= {n}000000")).collect::<Vec<_>>().join(", "))),
        ("deep", format!("run:\n    when ready then\n        when more then\n            return compute({args})\n    return 0\n")),
    ]
}

fn over(text: &str) -> Vec<usize> {
    text.lines().enumerate().filter(|(_, l)| l.chars().count() > WIDTH).map(|(n, _)| n + 1).collect()
}

#[test]
fn w_long_variants_wrap_to_the_width_and_keep_all_guarantees() {
    for (name, source) in long_variants() {
        assert!(common::parses(&source), "{name} must be valid");
        assert!(!over(&source).is_empty(), "{name} must start too long");
        let (text, gave_up) = ginfmt::format_detailed(&source, Options::default()).unwrap();
        assert!(!gave_up, "{name}: wrapping broke a guarantee:\n{text}");
        // Trailing comments are not code and are never broken.
        let code_over: Vec<_> = over(&text).into_iter().filter(|&n| !text.lines().nth(n - 1).unwrap().contains(" -- ")).collect();
        assert!(code_over.is_empty(), "{name}: lines {code_over:?} still over {WIDTH}:\n{text}");
        check_semantics(&source, &text).unwrap();
        check_comments(&source, &text).unwrap();
        check_idempotent(&fmt, &source).unwrap();
        assert_ne!(text, source, "{name}");
    }
}

#[test]
fn w_long_variants_scrambled_and_indented_deeper_still_wrap() {
    for (name, source) in long_variants() {
        for scrambled in scrambled(&source) {
            let text = fmt(&scrambled);
            assert!(over(&text).iter().all(|&n| text.lines().nth(n - 1).unwrap().contains(" -- ")), "{name}:\n{text}");
            check_semantics(&scrambled, &text).unwrap();
            check_idempotent(&fmt, &scrambled).unwrap();
        }
    }
}

#[test]
fn w_nothing_the_wrapper_could_still_break_is_left_long_in_the_corpus() {
    let mut left_long = 0;
    for input in corpus() {
        let text = fmt(&input.text);
        let breakable = ginfmt::breakable_lines(&text, WIDTH);
        assert!(breakable.is_empty(), "{}: lines {breakable:?} could still be broken", input.name);
        left_long += over(&text).len();
    }
    // Only what the parser cannot take on two lines: `where` claims, imports, a condition outside
    // brackets, long strings and comments.
    eprintln!("lines left over {WIDTH} columns: {left_long}");
}

#[test]
fn w_control_a_printer_that_does_not_wrap_is_caught_by_the_width_check() {
    let (_, source) = long_variants().remove(0);
    let unwrapped = ginfmt::format_with(&source, Options { width: None, ..Options::default() }).unwrap();
    assert!(!over(&unwrapped).is_empty());
    assert!(!ginfmt::breakable_lines(&unwrapped, WIDTH).is_empty());
    assert!(over(&fmt(&source)).is_empty());
}

#[test]
fn w_control_a_wrapper_that_changes_the_ast_is_caught() {
    // Breaking a conditional at the wrong level makes `then` a statement of its own.
    let source = "run:\n    result: when a_condition_that_is_long < 1 then a_value_that_is_long else another_value_that_is_long_enough_to_wrap_the_line\n    return result\n";
    let good = fmt(source);
    assert!(good.contains("\n        then "), "{good}");
    let broken = good.replace("\n        then ", "\nthen ").replace("\n        else ", "\nelse ");
    assert!(check_semantics(source, &broken).is_err());
    // Dropping an argument while wrapping.
    let args = names(9, "argument").join(", ");
    let source = format!("run:\n    result: compute({args})\n    return result\n");
    let good = fmt(&source);
    let broken: String = good.lines().filter(|l| !l.contains("argument_3")).map(|l| format!("{l}\n")).collect();
    assert!(check_semantics(&source, &broken).unwrap_err().contains("different tokens"));
    // Same tokens in another tree: a closing bracket moved up so the call ends early.
    let broken = good.replacen("        argument_8_with_a_longish_name\n    )", "    )\n        argument_8_with_a_longish_name", 1);
    assert!(check_semantics(&source, &broken).is_err());
}
