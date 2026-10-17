use super::*;
use parser::query::SourceParseExt;

fn inputs(body: &str) -> Vec<TypedFileAst> {
    let parsed = format!("{}{body}", test_fixtures::preludes::WORD_PRELUDE).parse_source_full();
    assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
    let mut typed = crate::transform::transform_file(parsed.ast, crate::FileId(0));
    for bind in typed.defs.values_mut() {
        bind.references = Default::default();
    }
    typed.exprs.facts.ownership_facts.references.fill(None);
    typed
        .exprs
        .facts
        .ownership_facts
        .reference_reads
        .fill(HashSet::new());
    let owners: HashMap<_, _> = typed
        .defs
        .keys()
        .enumerate()
        .map(|(index, id)| (*id, index % 3))
        .collect();
    (0..3)
        .map(|index| {
            let mut file = typed.clone();
            file.file_id = crate::FileId(index as u32);
            file.defs.retain(|id, _| owners[id] == index);
            file
        })
        .collect()
}

#[test]
fn parallel_rounds_preserve_reference_summaries_and_expression_facts() {
    for body in [
        "leaf(x ref Word) ref Word: return x\nforward(x ref Word) ref Word: return leaf(x)\nread(x ref Word) Word: return deref forward(x)\nbranch(x ref Word, flag Word) ref Word: return when flag is 0 then forward(x) else x\nlocal(flag Word) ref Word:\n    value Word: 7\n    return ref value\n",
        "first(x ref Word, flag Word) ref Word: return when flag is 0 then x else second(x, flag)\nsecond(x ref Word, flag Word) ref Word: return first(x, flag)\nmissing(x ref Word) ref Word: return absent(x)\n",
    ] {
        let original = inputs(body);
        let external = original
            .iter()
            .flat_map(|file| {
                file.defs
                    .iter()
                    .map(|(id, bind)| (*id, TypedCallableSignature::from(bind)))
            })
            .collect();
        for reverse in [false, true] {
            let mut input = original.clone();
            if reverse {
                input.reverse();
            }
            let mut expected = input.clone();
            let serial = infer_rounds(&mut expected, &external, false);
            assert!(serial.rounds > 1);
            for workers in [1, 2, 4] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(workers)
                    .build()
                    .unwrap();
                let mut actual = input.clone();
                let work = pool.install(|| infer_rounds(&mut actual, &external, true));
                assert_eq!(work.checked, serial.checked);
                assert_eq!(work.rounds, serial.rounds);
                for (left, right) in expected.iter().zip(&actual) {
                    assert!(crate::transform::check_session::exact_file_eq(left, right));
                }
            }
        }
    }
}
