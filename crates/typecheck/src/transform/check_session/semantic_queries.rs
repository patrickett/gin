use ast::FileAst;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;

use crate::{DefId, FileId, TypedCallableSignature, TypedFileAst};

use super::super::callable_signatures::CallableSignatureScope;

const ANSWERS_PER_FILE_MAX: usize = 128;

#[derive(Default)]
pub(in crate::transform) struct SourceQueries {
    entries: Mutex<HashMap<FileId, Vec<(Arc<FileAst>, bool)>>>,
}

impl SourceQueries {
    pub fn begin(&self) {
        for (_, used) in self.entries.lock().unwrap().values_mut().flatten() {
            *used = false;
        }
    }

    pub fn end(&self) {
        self.entries.lock().unwrap().retain(|_, entries| {
            entries.retain(|(_, used)| *used);
            !entries.is_empty()
        });
    }

    pub fn intern(&self, id: FileId, source: &FileAst) -> Arc<FileAst> {
        let mut entries = self.entries.lock().unwrap();
        intern_capped(
            entries.entry(id).or_default(),
            source,
            ANSWERS_PER_FILE_MAX,
            |answer, source| answer == source && super::same_ast_order(answer, source),
        )
    }
}

pub(in crate::transform) fn intern_capped<T: Clone>(
    entries: &mut Vec<(Arc<T>, bool)>,
    value: &T,
    cap: usize,
    same: impl Fn(&T, &T) -> bool,
) -> Arc<T> {
    if let Some((answer, used)) = entries.iter_mut().find(|(answer, _)| same(answer, value)) {
        *used = true;
        return Arc::clone(answer);
    }
    if entries.len() == cap {
        entries.remove(0);
    }
    let answer = Arc::new(value.clone());
    entries.push((Arc::clone(&answer), true));
    answer
}

pub(in crate::transform) struct FileQueries<K, V> {
    entries: Mutex<HashMap<FileId, Vec<Answer<K, V>>>>,
}

struct Answer<K, V> {
    input: TypedFileAst,
    key: K,
    reads: HashMap<DefId, Option<TypedCallableSignature>>,
    output: V,
    used: bool,
}

impl<K, V> Default for FileQueries<K, V> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl<K: PartialEq, V: Clone> FileQueries<K, V> {
    pub fn begin(&mut self) {
        for answers in self.entries.get_mut().unwrap().values_mut() {
            for answer in answers {
                answer.used = false;
            }
        }
    }

    pub fn end(&mut self) {
        self.entries.get_mut().unwrap().retain(|_, answers| {
            answers.retain(|answer| answer.used);
            !answers.is_empty()
        });
    }

    pub fn get(
        &self,
        input: &TypedFileAst,
        key: &K,
        signatures: &CallableSignatureScope<'_>,
    ) -> Option<V> {
        self.get_by(input, signatures, |stored| stored == key)
    }

    pub fn get_by(
        &self,
        input: &TypedFileAst,
        signatures: &CallableSignatureScope<'_>,
        matches: impl Fn(&K) -> bool,
    ) -> Option<V> {
        let mut entries = self.entries.lock().unwrap();
        let answers = entries.get_mut(&input.file_id)?;
        let answer = answers.iter_mut().find(|answer| {
            matches(&answer.key)
                && super::file_eq(&answer.input, input, false)
                && answer
                    .reads
                    .iter()
                    .all(|(id, value)| signatures.get(id) == value.as_ref())
        })?;
        answer.used = true;
        Some(answer.output.clone())
    }

    pub fn insert(
        &self,
        input: TypedFileAst,
        key: K,
        signatures: &CallableSignatureScope<'_>,
        lookups: impl IntoIterator<Item = DefId>,
        output: V,
    ) {
        let reads = lookups
            .into_iter()
            .map(|id| (id, signatures.get(&id).cloned()))
            .collect();
        let mut entries = self.entries.lock().unwrap();
        let answers = entries.entry(input.file_id).or_default();
        if answers.len() == ANSWERS_PER_FILE_MAX {
            answers.remove(0);
        }
        answers.push(Answer {
            input,
            key,
            reads,
            output,
            used: true,
        });
        assert!(answers.len() <= ANSWERS_PER_FILE_MAX);
    }
}
