use super::*;
use crate::transform::callable_signatures::SignatureReads;
use rayon::prelude::*;

#[derive(Default)]
pub struct EffectWork {
    pub checked: usize,
    pub skipped: usize,
    pub reused: usize,
}

#[cfg(test)]
pub(in crate::transform) fn infer(
    files: &mut [TypedFileAst],
    external: &HashMap<DefId, TypedCallableSignature>,
    parallel: bool,
) -> EffectWork {
    infer_cached(files, external, parallel, None)
}

pub(in crate::transform) type EffectQueries =
    crate::transform::check_session::FunctionQueries<EffectAnswer>;

#[derive(Clone)]
pub(in crate::transform) struct EffectAnswer {
    summary: CallableSummary,
    lookups: HashSet<DefId>,
}

pub(in crate::transform) fn infer_cached(
    files: &mut [TypedFileAst],
    external: &HashMap<DefId, TypedCallableSignature>,
    parallel: bool,
    queries: Option<&EffectQueries>,
) -> EffectWork {
    let _profile = compiler_profile::span(compiler_profile::Phase::Effects);
    let recursive_calls = recursion::recursive_calls(files.iter());
    let inputs: Option<Vec<_>> = queries.map(|queries| {
        files
            .iter()
            .map(|file| queries.inputs(file, Some(&recursive_calls)))
            .collect()
    });
    let slots: Vec<_> = files
        .iter()
        .enumerate()
        .flat_map(|(file, typed)| typed.defs.keys().map(move |id| (file, *id)))
        .collect();
    let winners: HashMap<_, _> = slots.iter().map(|(file, id)| (*id, *file)).collect();
    let mut signatures = CallableSignatureScope::new(
        external,
        files.iter().flat_map(|typed| {
            typed
                .defs
                .iter()
                .map(|(id, bind)| (*id, TypedCallableSignature::from(bind)))
        }),
    );
    let mut consumers: HashMap<DefId, HashSet<usize>> = HashMap::new();
    let mut scan_consumers = HashSet::new();
    let mut pending: Vec<_> = (0..slots.len()).collect();
    let mut work = EffectWork::default();
    while !pending.is_empty() {
        work.skipped += slots.len() - pending.len();
        let (shared, local) = signatures.inputs();
        let mut groups = vec![Vec::new(); files.len()];
        for &slot in &pending {
            let (file, id) = slots[slot];
            groups[file].push(id);
        }
        let profile_context = compiler_profile::Context::current();
        let infer_file = |(file, ids): (usize, &Vec<DefId>)| {
            let _profile_context = profile_context
                .as_ref()
                .map(compiler_profile::Context::enter);
            let signatures = CallableSignatureScope::borrowed(shared, local);
            let typed = &files[file];
            let output: Vec<_> = ids
                .iter()
                .map(|id| {
                    let input = inputs.as_ref().and_then(|inputs| inputs[file].get(id));
                    if let (Some(queries), Some((input, relocation))) = (queries, input) {
                        if let Some(mut answer) =
                            queries.get(typed, *id, input, *relocation, &signatures)
                        {
                            if relocation
                                .inverse()
                                .effects(&mut answer.summary.effects)
                                .is_some()
                            {
                                return (*id, answer.summary, answer.lookups, false, true);
                            }
                        }
                    }
                    let reads = SignatureReads::default();
                    let recording = input
                        .as_ref()
                        .map(|_| crate::transform::semantic_reads::SemanticReadGuard::begin());
                    let summary = infer_bind_summary(
                        typed,
                        &typed.defs[id],
                        &CallableSignatureScope::recorded(shared, local, &reads),
                        recursive_calls.get(id),
                    );
                    let (lookups, scanned) = reads.into_parts();
                    let types = recording.map(|recording| recording.finish());
                    if !scanned {
                        if let (Some(queries), Some((input, relocation))) = (queries, input) {
                            let mut answer = EffectAnswer {
                                summary: summary.clone(),
                                lookups: lookups.clone(),
                            };
                            let types = types.unwrap();
                            if !types.scanned
                                && relocation.effects(&mut answer.summary.effects).is_some()
                            {
                                queries.insert(
                                    typed,
                                    *id,
                                    (**input).clone(),
                                    *relocation,
                                    &signatures,
                                    lookups.clone(),
                                    types,
                                    answer,
                                );
                            }
                        }
                    }
                    (*id, summary, lookups, scanned, false)
                })
                .collect();
            (file, output)
        };
        let results: Vec<_> = if parallel {
            groups
                .par_iter()
                .enumerate()
                .filter(|(_, ids)| !ids.is_empty())
                .map(infer_file)
                .collect()
        } else {
            groups
                .iter()
                .enumerate()
                .filter(|(_, ids)| !ids.is_empty())
                .map(infer_file)
                .collect()
        };
        let mut updates = Vec::new();
        let slot_indices: HashMap<_, _> = slots
            .iter()
            .enumerate()
            .map(|(slot, pair)| (*pair, slot))
            .collect();
        for (file, output) in results {
            for (id, summary, lookups, scanned, reused) in output {
                if reused {
                    work.reused += 1;
                } else {
                    work.checked += 1;
                }
                let slot = slot_indices[&(file, id)];
                let bind = &files[file].defs[&id];
                for target in lookups {
                    consumers.entry(target).or_default().insert(slot);
                }
                if scanned {
                    scan_consumers.insert(slot);
                }
                if summary.effects != *bind.effects
                    || summary.param_requirements != bind.param_requirements
                    || summary.call_capability != bind.call_capability
                    || summary.definition_stage != bind.definition_stage
                {
                    updates.push((file, id, summary));
                }
            }
        }
        if updates.is_empty() {
            break;
        }
        let mut changed = HashSet::new();
        for (file, id, summary) in updates {
            let bind = files[file].defs.get_mut(&id).unwrap();
            bind.effects = crate::typed::Shared::new(summary.effects);
            bind.param_requirements = summary.param_requirements;
            bind.call_capability = summary.call_capability;
            bind.definition_stage = summary.definition_stage;
            if winners[&id] == file {
                signatures.replace(id, TypedCallableSignature::from(&*bind));
                changed.insert(id);
            }
        }
        let mut affected = HashSet::new();
        for id in &changed {
            if let Some(readers) = consumers.get(id) {
                affected.extend(readers.iter().copied());
            }
        }
        if !changed.is_empty() {
            affected.extend(scan_consumers.iter().copied());
        }
        pending = (0..slots.len())
            .filter(|slot| affected.contains(slot))
            .collect();
        if pending.is_empty() {
            work.skipped += slots.len();
        }
    }
    work
}

#[cfg(test)]
mod tests {
    use super::*;
    use parser::query::SourceParseExt;

    fn input(source: &str) -> TypedFileAst {
        let parsed = source.parse_source_full();
        assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
        let mut typed = crate::transform::transform_file(parsed.ast, crate::FileId(0));
        for bind in typed.defs.values_mut() {
            bind.effects = Default::default();
            bind.param_requirements.clear();
            bind.call_capability = CallCapability::RuntimeOnly;
            bind.definition_stage = DefinitionStage::RuntimeOnly;
        }
        typed
    }

    #[test]
    fn dependency_rounds_match_full_scan_and_skip_unaffected_functions() {
        let mut source = String::from(test_fixtures::preludes::WORD_PRELUDE);
        for id in 0..100 {
            source.push_str(&format!("leaf_{id}(x Word) Word: return x\n"));
        }
        source.push_str("first(x Word) Word: return leaf_0(x)\nsecond(x Word) Word: return first(x)\nthird(x Word) Word: return second(x)\n");
        let mut full = vec![input(&source)];
        let mut queries = full.clone();
        let external = HashMap::new();
        let expected = infer_package_full(&mut full, &external);
        let actual = infer(&mut queries, &external, false);
        assert!(
            actual.checked < expected.checked,
            "{} / {}",
            actual.checked,
            expected.checked
        );
        assert!(actual.skipped > 0);
        assert!(crate::transform::check_session::exact_file_eq(
            &full[0],
            &queries[0]
        ));
        for workers in [1, 2, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap();
            let mut parallel = vec![input(&source)];
            let work = pool.install(|| infer(&mut parallel, &external, true));
            assert_eq!(work.checked, actual.checked);
            assert!(crate::transform::check_session::exact_file_eq(
                &full[0],
                &parallel[0]
            ));
        }
    }

    #[test]
    fn recursion_missing_targets_and_signature_shadowing_match_full_scan() {
        let prefix = "Node has\n    next ref Node\n    finish(self):\n        return\n";
        for body in [
            "walk(value ref Node): return walk(value)\n",
            "walk(value ref Node): return bounce(value.next)\nbounce(value ref Node): return walk(value)\n",
            "walk(value ref Node): return missing(value)\n",
        ] {
            let original = input(&format!("{prefix}{body}"));
            let external = original
                .defs
                .iter()
                .map(|(id, bind)| (*id, TypedCallableSignature::from(bind)))
                .collect();
            for files in [
                vec![original.clone()],
                vec![original.clone(), original.clone()],
            ] {
                let mut full = files.clone();
                let mut queries = files;
                infer_package_full(&mut full, &external);
                infer(&mut queries, &external, false);
                for (left, right) in full.iter().zip(&queries) {
                    assert!(crate::transform::check_session::exact_file_eq(left, right));
                }
                for workers in [1, 2, 4] {
                    let pool = rayon::ThreadPoolBuilder::new()
                        .num_threads(workers)
                        .build()
                        .unwrap();
                    let mut parallel = vec![original.clone(); full.len()];
                    pool.install(|| infer(&mut parallel, &external, true));
                    for (left, right) in full.iter().zip(&parallel) {
                        assert!(crate::transform::check_session::exact_file_eq(left, right));
                    }
                }
            }
        }
    }
}
