//! The dependency picture of a session: which definition uses which, what a change would
//! affect, and a hash of each definition and of everything it depends on, so the session can
//! keep derived state (evaluated values, layouts) for definitions a declare did not touch.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};

use serde_json::{Value, json};
use typecheck::{CheckedSourceBodyKind as Kind, CheckedSourceCallee, CheckedSourceModule, SourceType};

use crate::check::Checked;
use crate::source::Decl;

#[derive(Debug, Clone)]
pub struct Def {
    pub key: String,
    pub name: String,
    pub kind: &'static str,
    pub text: String,
    /// Indexes of the definitions this one uses, and how that was found: `checker` (a call, a
    /// construction or a type in a signature or recorded body), `folded` (a call the checker
    /// replaced by its value; it remembers the callee) or `text` (the name is written in the
    /// definition; an over-approximation, only for what the checker records nothing of: a
    /// function whose body it declined, and types and constants).
    pub uses: BTreeMap<usize, &'static str>,
    pub mentions_target: bool,
}

pub struct Image {
    pub defs: Vec<Def>,
    /// `closure[i]` changes exactly when definition `i`, or anything it uses, changed.
    pub closure: Vec<u64>,
}

/// FNV-1a, 64 bits: fixed by definition, so a hash written to a snapshot means the same thing
/// when it is read back by another build (the standard library's hashers promise no such thing).
pub struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
}

pub fn fnv(bytes: &[u8]) -> u64 {
    let mut hasher = Fnv::default();
    hasher.write(bytes);
    hasher.finish()
}

fn hash_of(text: &str) -> u64 {
    fnv(text.as_bytes())
}

fn identifiers(text: &str) -> BTreeSet<&str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_')).filter(|w| !w.is_empty() && !w.starts_with(|c: char| c.is_ascii_digit())).collect()
}

impl Image {
    pub fn build(checked: &Checked) -> Self {
        let source = &checked.source;
        let mut defs: Vec<Def> = checked
            .decls
            .iter()
            .map(|d: &Decl| {
                let text = source[d.start..d.end].to_owned();
                Def { key: d.key.clone(), name: d.name.clone(), kind: d.kind, mentions_target: text.contains("#target"), text, uses: BTreeMap::new() }
            })
            .collect();
        // By name: every definition that is called `name`, a type's `has` blocks included.
        let mut by_name: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (index, d) in checked.decls.iter().enumerate() {
            by_name.entry(d.name.as_str()).or_default().push(index);
            for member in &d.members {
                by_name.entry(member.name.as_str()).or_default().push(index);
            }
        }
        // Text: a name written in a definition.
        for (index, def) in defs.iter_mut().enumerate() {
            let words = identifiers(&def.text);
            for word in words {
                for target in by_name.get(word).into_iter().flatten().copied().filter(|t| *t != index) {
                    def.uses.entry(target).or_insert("text");
                }
            }
        }
        // The checker: what the signatures and recorded bodies of its own functions name.
        if let Some(module) = &checked.module {
            let (edges, exact) = checker_edges(module, &by_name);
            for (index, target, via) in edges {
                if index != target {
                    defs[index].uses.insert(target, via);
                }
            }
            // A function whose body was recorded has every dependency in the checker's data
            // (signature types, calls, constructions, and calls folded into values): the names
            // merely written in its text add nothing.
            for index in exact {
                defs[index].uses.retain(|_, via| *via != "text");
            }
        }
        let mut closure = vec![0u64; defs.len()];
        for i in 0..defs.len() {
            closure[i] = closure_of(&defs, i, &mut Vec::new());
        }
        Self { defs, closure }
    }

    pub fn used_by(&self, index: usize) -> Vec<usize> {
        (0..self.defs.len()).filter(|i| self.defs[*i].uses.contains_key(&index)).collect()
    }

    /// Everything that would be affected by a change to `index`: its users, transitively.
    pub fn affects(&self, index: usize) -> Vec<usize> {
        let mut seen = BTreeSet::new();
        let mut pending = vec![index];
        while let Some(next) = pending.pop() {
            for user in self.used_by(next) {
                if seen.insert(user) {
                    pending.push(user);
                }
            }
        }
        seen.remove(&index);
        seen.into_iter().collect()
    }

    pub fn find(&self, name: &str) -> Vec<usize> {
        (0..self.defs.len()).filter(|i| self.defs[*i].name == name || self.defs[*i].key == name).collect()
    }

    /// A hash that changes when any definition called `name`, or anything they use, changes.
    pub fn closure_of_name(&self, name: &str) -> u64 {
        let mut hasher = Fnv::default();
        for i in self.find(name) {
            self.closure[i].hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn json(&self, filter: Option<&dyn Fn(usize) -> Option<bool>>) -> Value {
        let defs: Vec<Value> = (0..self.defs.len())
            .map(|i| {
                let d = &self.defs[i];
                let mut value = json!({
                    "key": d.key,
                    "name": d.name,
                    "kind": d.kind,
                    "uses": d.uses.iter().map(|(t, via)| json!({"key": self.defs[*t].key, "via": via})).collect::<Vec<_>>(),
                    "used_by": self.used_by(i).iter().map(|u| self.defs[*u].key.clone()).collect::<Vec<_>>(),
                    "affects": self.affects(i).iter().map(|u| self.defs[*u].key.clone()).collect::<Vec<_>>(),
                    "mentions_target": d.mentions_target,
                });
                if let Some(cached) = filter.and_then(|f| f(i)) {
                    value["evaluated_state_kept"] = json!(cached);
                }
                value
            })
            .collect();
        json!(defs)
    }
}

fn closure_of(defs: &[Def], index: usize, stack: &mut Vec<usize>) -> u64 {
    if stack.contains(&index) {
        return 0;
    }
    stack.push(index);
    let mut hasher = Fnv::default();
    hash_of(&defs[index].text).hash(&mut hasher);
    for target in defs[index].uses.keys() {
        closure_of(defs, *target, stack).hash(&mut hasher);
    }
    stack.pop();
    hasher.finish()
}

/// Edges (from, to, how) from the checker's data, and the definitions that data covers fully.
fn checker_edges(
    module: &CheckedSourceModule,
    by_name: &BTreeMap<&str, Vec<usize>>,
) -> (Vec<(usize, usize, &'static str)>, BTreeSet<usize>) {
    let file = module.checked();
    let owners = |name: &str| by_name.get(name).cloned().unwrap_or_default();
    let schema_owners = |schema: usize| {
        let name = &file.aggregates[schema].name;
        owners(name.split('.').next().unwrap_or(name))
    };
    let type_owners = |ty: SourceType, out: &mut Vec<usize>| {
        if let SourceType::Aggregate { id } | SourceType::Sum { id } = ty {
            if let Some(typecheck::CheckedSourceInstanceHead::Nominal { schema }) = file.instances.get(id as usize).map(|i| i.head) {
                out.extend(schema_owners(schema as usize));
            }
        }
    };
    let mut edges = Vec::new();
    let mut exact = BTreeSet::new();
    let name_of = |callee: CheckedSourceCallee| match callee {
        CheckedSourceCallee::Local { function } => file.functions.get(function as usize).map(|f| f.name.as_str()),
        CheckedSourceCallee::Dependency { .. } => None,
    };
    for (index, function) in file.functions.iter().enumerate() {
        if module.function_home(index).is_none_or(|(home, own)| home != module.name() || own as usize != index) {
            continue;
        }
        let from = owners(&function.name);
        if file.bodies[index].is_some() {
            exact.extend(from.iter().copied());
        }
        let mut to = Vec::new();
        function.parameters.iter().chain([&function.result]).for_each(|t| type_owners(*t, &mut to));
        if let Some(body) = &file.bodies[index] {
            for node in &body.expressions {
                type_owners(node.ty, &mut to);
                match &node.kind {
                    Kind::Call { callee: CheckedSourceCallee::Local { function }, .. } => {
                        if let Some(f) = file.functions.get(*function as usize) {
                            to.extend(owners(&f.name));
                        }
                    }
                    Kind::Construct { schema, .. } => to.extend(schema_owners(*schema as usize)),
                    _ => {}
                }
            }
        }
        for f in &from {
            for t in &to {
                edges.push((*f, *t, "checker"));
            }
        }
    }
    // Calls the recorder replaced by their value leave no call in the body: the checker kept
    // which callee each one was.
    for (caller, callee) in &file.folds {
        let Some(caller) = file.functions.get(*caller as usize) else { continue };
        for f in owners(&caller.name) {
            for t in name_of(*callee).map(owners).unwrap_or_default() {
                edges.push((f, t, "folded"));
            }
        }
    }
    (edges, exact)
}
