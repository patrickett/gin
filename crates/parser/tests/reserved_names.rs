use diagnostic::Category;
use parser::query::SourceParseExt;

#[test]
fn reserved_names_report_flaws_at_the_keyword() {
    for token in lexer::Token::KEYWORDS {
        let keyword = token.keyword().unwrap();
        for source in [
            format!("{keyword}: 0\n"),
            format!("{keyword} Int: 0\n"),
            format!("{keyword}(): return 0\n"),
            format!("f() value Int, {keyword} Int: return 0\n"),
            format!("f():\n  {keyword}: 0\n  return 0\n"),
        ] {
            let output = source.parse_source_full();
            let errors: Vec<_> = output
                .symptoms
                .iter()
                .filter(|d| d.code.slug() == "parse-reserved-name")
                .collect();
            assert_eq!(errors.len(), 1, "{source}: {:?}", output.symptoms);
            let error = errors[0];
            assert_eq!(error.category, Category::Flaw);
            assert_eq!(&source[error.span.start()..error.span.end()], keyword);
            assert!(error.message.contains(keyword));
            assert!(
                error
                    .help
                    .as_ref()
                    .unwrap()
                    .contains("choose a different name")
            );
        }
    }
}

#[test]
fn reference_results_and_ownership_conventions_remain_valid() {
    for source in [
        "f(self): return self",
        "private\nhidden: 0\n",
        "use 'helpers' Thing\n",
        "f(value Int) Int: return value\n",
        "ref(1)",
        "mut(1)",
        "f(value ref Int) ref Int: return value",
        "f(value mut Int) mut Int: return value",
        "f(value own Int) result Int: return value",
        "f(value ref Int) result ref Int: return value",
        "f(value mut Int) result mut Int: return value",
        "f(value Int):\n  value\n  return value\n",
    ] {
        let output = source.parse_source_full();
        assert!(
            !output.symptoms.iter().any(|d| d.category == Category::Flaw),
            "{source}: {:?}",
            output.symptoms
        );
    }
}

#[test]
fn non_type_keywords_cannot_name_the_first_result() {
    for keyword in ["own", "return"] {
        for source in [format!("f() {keyword} Int: return 0"), format!("f() {keyword}: return 0")] {
            let output = source.parse_source_full();
            assert!(
                output
                    .symptoms
                    .iter()
                    .any(|d| d.code.slug() == "parse-reserved-name"),
                "{source}: {:?}",
                output.symptoms
            );
        }
    }
}

#[test]
fn keywords_cannot_name_parameters_except_the_receiver() {
    for token in lexer::Token::KEYWORDS {
        if *token == lexer::Token::SelfInstance {
            continue;
        }
        let keyword = token.keyword().unwrap();
        let source = format!("f({keyword} Int): return 0");
        let output = source.parse_source_full();
        assert!(
            output
                .symptoms
                .iter()
                .any(|d| d.code.slug() == "parse-reserved-name"),
            "{source}: {:?}",
            output.symptoms
        );
    }
}
