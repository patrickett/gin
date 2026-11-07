//! The semver classifier over two signature layers: each public item is
//! Unchanged, Compatible, Unknown or Breaking with a reason,
//! the package result is the largest, and over several targets the largest bump wins.
//!
//! Claims are compared syntactically, as sets of canonical encodings: a precondition set that
//! only loses members is weakened, one that only gains is strengthened, anything else is
//! Unknown (no prover is consulted). Ownership summaries are compared strictly:
//! any difference but wider input ranges is Breaking.
use super::*;
use std::collections::BTreeSet;
use std::fmt;

/// How a change affects dependents. Ordered: the largest wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Compatibility {
    Unchanged,
    /// Dependents still check: a minor bump.
    Compatible,
    /// The classifier cannot decide; the publisher must.
    Unknown,
    /// Some dependent may stop checking: a major bump.
    Breaking,
}

impl fmt::Display for Compatibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unchanged => "unchanged",
            Self::Compatible => "compatible",
            Self::Unknown => "unknown",
            Self::Breaking => "breaking",
        })
    }
}

/// One item's verdict and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemChange {
    /// `function double`, `type Digit`, `constant limit`, `target x86_64-...`.
    pub item: String,
    pub result: Compatibility,
    pub reasons: Vec<String>,
}

impl fmt::Display for ItemChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}: {}", self.result, self.item, self.reasons.join("; "))
    }
}

/// The verdict over a whole package (one or several targets).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub result: Compatibility,
    /// Every item that is not unchanged, plus unchanged items with a note (a layout-only
    /// change), in a deterministic order.
    pub changes: Vec<ItemChange>,
}

/// Compare two artifacts of one module for one target. Artifacts of another schema version
/// classify as Unknown, as the legacy classifier does.
pub fn classify_artifacts(old: &[u8], new: &[u8]) -> Result<Classification, ArtifactError> {
    let decode = |bytes| match interface_codec::decode_artifact(bytes) {
        Ok(artifact) => Ok(Some(artifact)),
        Err(CodecError::Version(_)) => Ok(None),
        Err(error) => Err(ArtifactError::Codec(error)),
    };
    let (Some(old), Some(new)) = (decode(old)?, decode(new)?) else {
        return Ok(Classification {
            result: Compatibility::Unknown,
            changes: vec![ItemChange {
                item: "artifact".into(),
                result: Compatibility::Unknown,
                reasons: vec!["schema version differs".into()],
            }],
        });
    };
    let own = |artifact: &interface_codec::Artifact| {
        let id = &artifact.header().module;
        (id.package.clone(), id.module.clone())
    };
    if own(&old) != own(&new) {
        return Err(ArtifactError::WrongModule(new.header().module.module.clone()));
    }
    let old = Surface::of(&old)?;
    let new = Surface::of(&new)?;
    Ok(compare(&old, &new))
}

/// Compare per-target artifacts (`(target, bytes)`): each target present in both is compared,
/// an added target is Compatible, a dropped one Breaking, and the largest result wins.
pub fn classify_targets(old: &[(String, Vec<u8>)], new: &[(String, Vec<u8>)]) -> Result<Classification, ArtifactError> {
    let olds: BTreeMap<&str, &[u8]> = old.iter().map(|(t, b)| (t.as_str(), b.as_slice())).collect();
    let news: BTreeMap<&str, &[u8]> = new.iter().map(|(t, b)| (t.as_str(), b.as_slice())).collect();
    if olds.len() != old.len() || news.len() != new.len() {
        return Err(shape("a target listed twice"));
    }
    let mut changes = Vec::new();
    let mut result = Compatibility::Unchanged;
    let targets: BTreeSet<&str> = olds.keys().chain(news.keys()).copied().collect();
    for target in targets {
        match (olds.get(target), news.get(target)) {
            (Some(a), Some(b)) => {
                let one = classify_artifacts(a, b)?;
                result = result.max(one.result);
                for mut change in one.changes {
                    change.item = format!("{} [{target}]", change.item);
                    changes.push(change);
                }
            }
            (None, Some(_)) => {
                result = result.max(Compatibility::Compatible);
                changes.push(ItemChange { item: format!("target {target}"), result: Compatibility::Compatible, reasons: vec!["added".into()] });
            }
            (Some(_), None) => {
                result = Compatibility::Breaking;
                changes.push(ItemChange { item: format!("target {target}"), result: Compatibility::Breaking, reasons: vec!["dropped".into()] });
            }
            (None, None) => unreachable!("a target comes from one of the maps"),
        }
    }
    Ok(Classification { result, changes })
}

/// A signature layer with every type number replaced by the type it numbers and the module's
/// own identity reduced to package and path, so two versions compare item by item.
struct Surface {
    /// By `package/module::name` (the home's version too when it is a dependency's), with the
    /// key's encoding for the private-field lookup.
    layouts: BTreeMap<String, (Vec<u8>, Vec<Node>)>,
    /// Private fields per layout (from `inst`), for the layout-only rows.
    private: BTreeMap<Vec<u8>, Node>,
    functions: BTreeMap<String, Vec<Node>>,
    exports: BTreeMap<String, Node>,
    constants: BTreeMap<String, Node>,
    aliases: BTreeMap<String, Node>,
    traits: BTreeMap<String, (String, Node)>,
    accessors: BTreeSet<Vec<u8>>,
}

fn bytes(node: &Node) -> Vec<u8> {
    let mut out = Vec::new();
    interface_codec::encode_node(&mut out, node);
    out
}

/// Expand the type numbers of a layer and normalize module identities.
struct Expander<'a> {
    table: &'a [Node],
    own: &'a ModuleId,
    done: Vec<Option<Node>>,
}

impl Expander<'_> {
    fn instance(&mut self, number: &Node) -> Result<Node, ArtifactError> {
        let at = index(number)?;
        if at >= self.table.len() {
            return Err(shape("type number"));
        }
        if let Some(done) = &self.done[at] {
            return Ok(done.clone());
        }
        // An entry names only entries before it, so this recursion ends.
        self.done[at] = Some(Node::Absent);
        let expanded = self.node(&self.table[at].clone())?;
        self.done[at] = Some(expanded.clone());
        Ok(expanded)
    }

    fn node(&mut self, node: &Node) -> Result<Node, ArtifactError> {
        Ok(match node {
            Node::Record(tag @ (TY_AGGREGATE | TY_SUM | TY_ARRAY | TY_SEQUENCE | TY_POINTER), fields) if fields.len() == 1 => {
                Node::Record(*tag, vec![self.instance(&fields[0])?])
            }
            Node::Record(OUTPUT, fields) if fields.len() == 11 => {
                let mut out: Vec<Node> = fields.iter().map(|f| self.node(f)).collect::<Result<_, _>>()?;
                if !matches!(fields[6], Node::Absent) {
                    out[6] = self.instance(&fields[6])?;
                }
                Node::Record(OUTPUT, out)
            }
            Node::Record(tag, fields) => {
                if *tag == 2 {
                    if let Ok(id) = ModuleId::from_node(node) {
                        return Ok(self.module(id).to_node());
                    }
                }
                Node::Record(*tag, fields.iter().map(|f| self.node(f)).collect::<Result<_, _>>()?)
            }
            Node::List(items) => Node::List(items.iter().map(|f| self.node(f)).collect::<Result<_, _>>()?),
            other => other.clone(),
        })
    }

    /// The module itself loses version and digest (they change with every release); a
    /// dependency keeps its version, since two versions' types never unify.
    fn module(&self, mut id: ModuleId) -> ModuleId {
        id.source = [0; 32];
        if id.package == self.own.package && id.module == self.own.module {
            id.version = String::new();
        }
        id
    }
}

impl Surface {
    fn of(artifact: &interface_codec::Artifact) -> Result<Self, ArtifactError> {
        let sig = record(&artifact.signature.body, SIG, 9)?;
        let inst = record(&artifact.instantiation.body, INST, 6)?;
        let own = &artifact.header().module;
        let sig_table = list(&sig[0])?;
        let mut x = Expander { table: sig_table, own, done: vec![None; sig_table.len()] };
        let mut layouts = BTreeMap::new();
        for node in list(&sig[1])? {
            let node = x.node(node)?;
            let fields = record(&node, LAYOUT, 18)?.to_vec();
            let (home, name) = item_key(&fields[0])?;
            let version = if home.version.is_empty() { String::new() } else { format!("@{}", home.version) };
            let label = format!("{}{version}/{}::{name}", home.package, home.module);
            layouts.insert(label, (bytes(&fields[0]), fields));
        }
        let mut functions = BTreeMap::new();
        for node in list(&sig[2])? {
            let node = x.node(node)?;
            let fields = record(&node, FUNCTION, 14)?.to_vec();
            functions.insert(string(&fields[0])?, fields);
        }
        let mut exports = BTreeMap::new();
        for node in list(&sig[3])? {
            let node = x.node(node)?;
            let fields = record(&node, EXPORT, 3)?;
            exports.insert(string(&fields[0])?, Node::List(fields[1..].to_vec()));
        }
        let named = |x: &mut Expander<'_>, node: &Node, tag: u64| -> Result<BTreeMap<String, Node>, ArtifactError> {
            let mut out = BTreeMap::new();
            for item in list(node)? {
                let item = x.node(item)?;
                let fields = record(&item, tag, 2)?;
                out.insert(string(&fields[0])?, fields[1].clone());
            }
            Ok(out)
        };
        let constants = named(&mut x, &sig[4], CONSTANT)?;
        let aliases = named(&mut x, &sig[5], ALIAS)?;
        let mut traits = BTreeMap::new();
        for node in list(&sig[6])? {
            let node = x.node(node)?;
            let fields = record(&node, TRAIT, 3)?;
            traits.insert(string(&fields[0])?, (string(&fields[1])?, fields[2].clone()));
        }
        let mut accessors = BTreeSet::new();
        for node in list(&sig[7])?.iter().chain(list(&sig[8])?) {
            accessors.insert(bytes(node));
        }
        let inst_table = list(&inst[0])?;
        let mut y = Expander { table: inst_table, own, done: vec![None; inst_table.len()] };
        let mut private = BTreeMap::new();
        for node in list(&inst[1])? {
            let node = y.node(node)?;
            let fields = record(&node, PRIVATE_FIELDS, 2)?;
            private.insert(bytes(&fields[0]), fields[1].clone());
        }
        Ok(Self { layouts, private, functions, exports, constants, aliases, traits, accessors })
    }
}

/// Collects one item's reasons and its largest result.
struct Verdict {
    result: Compatibility,
    reasons: Vec<String>,
}

impl Verdict {
    fn new() -> Self {
        Self { result: Compatibility::Unchanged, reasons: Vec::new() }
    }

    fn say(&mut self, result: Compatibility, reason: impl Into<String>) {
        self.result = self.result.max(result);
        self.reasons.push(reason.into());
    }

    fn into_change(self, item: String) -> Option<ItemChange> {
        (!self.reasons.is_empty()).then_some(ItemChange { item, result: self.result, reasons: self.reasons })
    }
}

fn compare(old: &Surface, new: &Surface) -> Classification {
    let mut changes = Vec::new();
    added_removed(&mut changes, "export", &old.exports, &new.exports, |verdict, a, b| {
        if a != b {
            verdict.say(Compatibility::Breaking, "now names another item");
        }
    });
    added_removed(&mut changes, "function", &old.functions, &new.functions, |verdict, a, b| function(verdict, a, b));
    added_removed(&mut changes, "type", &old.layouts, &new.layouts, |verdict, a, b| {
        layout(verdict, &a.1, &b.1, old.private.get(&a.0), new.private.get(&b.0));
    });
    added_removed(&mut changes, "constant", &old.constants, &new.constants, |verdict, a, b| {
        if a != b {
            verdict.say(Compatibility::Breaking, "value changed");
        }
    });
    added_removed(&mut changes, "alias", &old.aliases, &new.aliases, |verdict, a, b| {
        if a != b {
            verdict.say(Compatibility::Breaking, "target changed");
        }
    });
    added_removed(&mut changes, "trait", &old.traits, &new.traits, |verdict, a, b| trait_change(verdict, a, b));
    let accessors_old: BTreeMap<Vec<u8>, ()> = old.accessors.iter().map(|a| (a.clone(), ())).collect();
    let accessors_new: BTreeMap<Vec<u8>, ()> = new.accessors.iter().map(|a| (a.clone(), ())).collect();
    for (key, _) in &accessors_old {
        if !accessors_new.contains_key(key) {
            changes.push(ItemChange { item: "accessor".into(), result: Compatibility::Breaking, reasons: vec!["removed".into()] });
        }
    }
    for (key, _) in &accessors_new {
        if !accessors_old.contains_key(key) {
            changes.push(ItemChange { item: "accessor".into(), result: Compatibility::Compatible, reasons: vec!["added".into()] });
        }
    }
    let result = changes.iter().map(|change| change.result).max().unwrap_or(Compatibility::Unchanged);
    Classification { result, changes }
}

fn added_removed<K: Ord + fmt::Display, V: PartialEq>(
    changes: &mut Vec<ItemChange>,
    kind: &str,
    old: &BTreeMap<K, V>,
    new: &BTreeMap<K, V>,
    mut both: impl FnMut(&mut Verdict, &V, &V),
) {
    for (name, a) in old {
        let item = format!("{kind} {name}");
        match new.get(name) {
            None => changes.push(ItemChange { item, result: Compatibility::Breaking, reasons: vec!["removed".into()] }),
            Some(b) => {
                let mut verdict = Verdict::new();
                if a != b {
                    both(&mut verdict, a, b);
                    if verdict.reasons.is_empty() {
                        // A difference no rule names: never call it unchanged.
                        verdict.say(Compatibility::Unknown, "changed");
                    }
                }
                changes.extend(verdict.into_change(item));
            }
        }
    }
    for name in new.keys() {
        if !old.contains_key(name) {
            changes.push(ItemChange { item: format!("{kind} {name}"), result: Compatibility::Compatible, reasons: vec!["added".into()] });
        }
    }
}

/// Claims as a set of canonical encodings.
fn claim_set(node: &Node) -> BTreeSet<Vec<u8>> {
    match node {
        Node::List(items) => items.iter().map(bytes).collect(),
        _ => BTreeSet::new(),
    }
}

/// `requires`: preconditions (fewer is better for callers) or postconditions (more is better).
fn claims(verdict: &mut Verdict, what: &str, old: &Node, new: &Node, precondition: bool) {
    if old == new {
        return;
    }
    let (a, b) = (claim_set(old), claim_set(new));
    if a == b {
        // Same claims, other order: whether order is semantic to the prover is open.
        verdict.say(Compatibility::Unknown, format!("{what} reordered"));
    } else if b.is_subset(&a) {
        let result = if precondition { Compatibility::Compatible } else { Compatibility::Breaking };
        verdict.say(result, format!("{what} weakened"));
    } else if a.is_subset(&b) {
        let result = if precondition { Compatibility::Breaking } else { Compatibility::Compatible };
        verdict.say(result, format!("{what} strengthened"));
    } else {
        verdict.say(Compatibility::Unknown, format!("{what} changed (not decided syntactically)"));
    }
}

/// Generic bounds: `None` is unbounded.
fn bounds(verdict: &mut Verdict, old: &Node, new: &Node) {
    let (Ok(a), Ok(b)) = (list(old), list(new)) else { return };
    for (a, b) in a.iter().zip(b) {
        match (a, b) {
            _ if a == b => {}
            (Node::Absent, _) => verdict.say(Compatibility::Breaking, "generic bound strengthened"),
            (_, Node::Absent) => verdict.say(Compatibility::Compatible, "generic bound weakened"),
            _ => verdict.say(Compatibility::Unknown, "generic bound changed"),
        }
    }
}

fn function(verdict: &mut Verdict, old: &[Node], new: &[Node]) {
    let (Ok(s), Ok(t)) = (record(&old[1], SIGNATURE, 12), record(&new[1], SIGNATURE, 12)) else {
        verdict.say(Compatibility::Unknown, "malformed signature");
        return;
    };
    let breaking = |verdict: &mut Verdict, at: usize, what: &str| {
        if s[at] != t[at] {
            verdict.say(Compatibility::Breaking, what.to_owned());
        }
    };
    breaking(verdict, 0, "memory operation changed");
    breaking(verdict, 1, "trait provision changed");
    breaking(verdict, 2, "instruction changed");
    breaking(verdict, 3, "literal parameters changed");
    breaking(verdict, 4, "operator changed");
    breaking(verdict, 7, "parameters changed");
    breaking(verdict, 8, "`mut` parameters changed");
    breaking(verdict, 11, "result type changed");
    if s[9] != t[9] {
        verdict.say(Compatibility::Breaking, "generic parameters changed");
    } else {
        bounds(verdict, &s[10], &t[10]);
    }
    if old[2] != new[2] && s[7] == t[7] {
        verdict.say(Compatibility::Breaking, "parameters changed");
    }
    if old[3] != new[3] {
        verdict.say(Compatibility::Breaking, "convention changed (ref/mut/own)");
    }
    claims(verdict, "precondition", &old[4], &new[4], true);
    claims(verdict, "postcondition", &old[5], &new[5], false);
    if old[6] != new[6] {
        variant_claims(verdict, &old[6], &new[6]);
    }
    if old[7] != new[7] {
        verdict.say(Compatibility::Breaking, "`::` transitions changed");
    }
    if old[8] != new[8] {
        verdict.say(Compatibility::Breaking, "implicit type parameters changed");
    }
    if old[9] != new[9] {
        summaries(verdict, &old[9], &new[9]);
    }
    if old[10] != new[10] {
        verdict.say(Compatibility::Breaking, "effects on `mut` places changed");
    }
    if old[11] != new[11] || old[12] != new[12] {
        verdict.say(Compatibility::Breaking, "result borrow changed");
    }
    match (&old[13], &new[13]) {
        (a, b) if a == b => {}
        (Node::Absent, _) => verdict.say(Compatibility::Compatible, "bound query added"),
        _ => verdict.say(Compatibility::Breaking, "bound query removed or changed"),
    }
}

fn variant_claims(verdict: &mut Verdict, old: &Node, new: &Node) {
    let split = |node: &Node| -> BTreeMap<Vec<u8>, Node> {
        list(node)
            .unwrap_or(&[])
            .iter()
            .filter_map(|item| record(item, VARIANT_CLAIMS, 2).ok().map(|f| (bytes(&f[0]), f[1].clone())))
            .collect()
    };
    let (a, b) = (split(old), split(new));
    if a.keys().ne(b.keys()) {
        verdict.say(Compatibility::Breaking, "result variants changed");
        return;
    }
    for (member, claims_a) in &a {
        claims(verdict, "variant postcondition", claims_a, &b[member], false);
    }
}

/// Strict (D5): equal, or equal but for input ranges that only widen.
fn summaries(verdict: &mut Verdict, old: &Node, new: &Node) {
    let strip = |node: &Node| -> Option<(Vec<Node>, Vec<Node>)> {
        let mut rest = Vec::new();
        let mut ranges = Vec::new();
        for summary in list(node).ok()? {
            let mut fields = record(summary, SUMMARY, 7).ok()?.to_vec();
            ranges.push(std::mem::replace(&mut fields[6], Node::Absent));
            rest.push(Node::Record(SUMMARY, fields));
        }
        Some((rest, ranges))
    };
    let (Some((a, ra)), Some((b, rb))) = (strip(old), strip(new)) else {
        verdict.say(Compatibility::Breaking, "ownership summaries changed");
        return;
    };
    if a != b {
        verdict.say(Compatibility::Breaking, "ownership summaries changed");
        return;
    }
    let range = |node: &Node| -> Option<(usize, I256, I256)> {
        let (input, range) = unpair(node).ok()?;
        let (low, high) = unpair(range).ok()?;
        Some((index(input).ok()?, int(low).ok()?, int(high).ok()?))
    };
    let widened = ra.iter().zip(&rb).all(|(a, b)| {
        let (Ok(a), Ok(b)) = (list(a), list(b)) else { return false };
        a.len() == b.len()
            && a.iter().zip(b).all(|(a, b)| match (range(a), range(b)) {
                (Some((i, lo, hi)), Some((j, lo2, hi2))) => i == j && lo2 <= lo && hi2 >= hi,
                _ => false,
            })
    });
    if widened {
        verdict.say(Compatibility::Compatible, "summary input ranges widened");
    } else {
        verdict.say(Compatibility::Breaking, "summary input ranges narrowed or changed");
    }
}

/// A field compared without its position: a private field moving the public ones is layout.
fn public_fields(node: &Node) -> Vec<Node> {
    list(node)
        .unwrap_or(&[])
        .iter()
        .map(|field| match field {
            Node::Record(FIELD, f) if f.len() == 7 => Node::Record(FIELD, f[1..].to_vec()),
            other => other.clone(),
        })
        .collect()
}

fn layout(verdict: &mut Verdict, old: &[Node], new: &[Node], private_old: Option<&Node>, private_new: Option<&Node>) {
    let flag = |verdict: &mut Verdict, at: usize, result: Compatibility, what: &str| {
        if old[at] != new[at] {
            verdict.say(result, what.to_owned());
        }
    };
    // Generic parameters: one added with a default keeps every old use valid.
    if old[2] != new[2] {
        let (a, b) = (list(&old[2]).unwrap_or(&[]), list(&new[2]).unwrap_or(&[]));
        let defaults = list(&new[10]).unwrap_or(&[]);
        let added_with_defaults = b.len() > a.len()
            && b.starts_with(a)
            && (a.len()..b.len()).all(|at| defaults.get(at).is_some_and(|d| !matches!(d, Node::Absent)));
        if added_with_defaults {
            verdict.say(Compatibility::Compatible, "generic parameter added with a default");
        } else {
            verdict.say(Compatibility::Breaking, "generic parameters changed");
        }
    } else if old[10] != new[10] {
        verdict.say(Compatibility::Breaking, "generic parameter default changed");
    }
    bounds(verdict, &old[11], &new[11]);
    // Fields: the public ones and how many are hidden.
    let (hidden_a, hidden_b) = (index(&old[4]).unwrap_or(0), index(&new[4]).unwrap_or(0));
    let (fa, fb) = (public_fields(&old[3]), public_fields(&new[3]));
    if fa != fb {
        let kept = fa.iter().all(|field| fb.contains(field)) && fb.len() > fa.len();
        let order_kept = fb.iter().filter(|field| fa.contains(field)).eq(fa.iter());
        if kept && order_kept && hidden_a > 0 {
            verdict.say(Compatibility::Compatible, "public field added to a type with private fields");
        } else if kept && order_kept {
            verdict.say(Compatibility::Breaking, "public field added to a type dependents construct");
        } else {
            verdict.say(Compatibility::Breaking, "public field removed, renamed, retyped or reordered");
        }
    }
    match (hidden_a, hidden_b) {
        (a, b) if a == b => {}
        (0, _) => verdict.say(Compatibility::Breaking, "private fields added: dependents can no longer build it"),
        (_, 0) => verdict.say(Compatibility::Compatible, "private fields removed"),
        _ => {}
    }
    if old[3] != new[3] && fa == fb || old[5] != new[5] || private_old != private_new {
        // Only the hidden part moved: layout only, unless the layout is observable.
        // TODO(layout-compatibility): decide the semver promise for layout queries and register/FFI
        // exposure before classifying these changes as compatible or breaking instead of Unknown.
        let registers = |fields: &Node| {
            list(fields).unwrap_or(&[]).iter().any(|f| matches!(f, Node::Record(FIELD, f) if f.len() == 7 && !matches!(f[3], Node::Absent)))
        };
        if registers(&old[3]) || registers(&new[3]) {
            verdict.say(Compatibility::Unknown, "layout of a `#register` type changed (decision D4)");
        } else if fa == fb && hidden_a > 0 && hidden_b > 0 {
            verdict.say(Compatibility::Unchanged, "private fields changed: layout only");
        }
    }
    if old[6] != new[6] {
        let (a, b) = (list(&old[6]).unwrap_or(&[]), list(&new[6]).unwrap_or(&[]));
        let what = if b.len() > a.len() && a.iter().all(|v| b.contains(v)) {
            "union variant added"
        } else if b.len() < a.len() && b.iter().all(|v| a.contains(v)) {
            "union variant removed"
        } else {
            "union variants changed or reordered"
        };
        verdict.say(Compatibility::Breaking, what);
    }
    match (&old[7], &new[7]) {
        (a, b) if a == b => {}
        (_, b) if *b == enc_capability(Capability::Unrestricted) => verdict.say(Compatibility::Compatible, "`#unrestricted` added"),
        _ => verdict.say(Compatibility::Breaking, "`#unrestricted` removed"),
    }
    flag(verdict, 8, Compatibility::Breaking, "kind changed");
    flag(verdict, 9, Compatibility::Breaking, "`#wrap` added or removed");
    if old[12] != new[12] {
        let (a, b) = (list(&old[12]).unwrap_or(&[]), list(&new[12]).unwrap_or(&[]));
        if a.iter().all(|t| b.contains(t)) {
            verdict.say(Compatibility::Compatible, "trait provision added");
        } else {
            verdict.say(Compatibility::Breaking, "trait provision removed");
        }
    }
    flag(verdict, 13, Compatibility::Breaking, "borrowing changed");
    if old[14] != new[14] {
        let range = |node: &Node| list(node).ok().and_then(|items| items.first().cloned());
        let pair_of = |node: Option<Node>| -> Option<(I256, I256)> {
            let node = node?;
            let (lo, hi) = unpair(&node).ok()?;
            Some((int(lo).ok()?, int(hi).ok()?))
        };
        let what = match (pair_of(range(&old[14])), pair_of(range(&new[14]))) {
            (Some((lo, hi)), Some((lo2, hi2))) if lo2 <= lo && hi2 >= hi => "domain bounds widened (breaking by default: no position analysis)",
            (Some((lo, hi)), Some((lo2, hi2))) if lo2 >= lo && hi2 <= hi => "domain bounds narrowed",
            _ => "domain bounds changed",
        };
        verdict.say(Compatibility::Breaking, what);
    }
    flag(verdict, 15, Compatibility::Breaking, "domain claims changed");
    flag(verdict, 16, Compatibility::Breaking, "discriminant changed");
    flag(verdict, 17, Compatibility::Breaking, "refinement claims changed");
}

/// A trait's methods and whether each has a default body, from its declaration text.
fn trait_methods(text: &str) -> Option<BTreeMap<String, (bool, String)>> {
    let source = parser::parse_strict(text).ok()?;
    let mut methods = BTreeMap::new();
    for item in &source.items {
        let ast::Item::Type(declaration) = &item.value else { continue };
        let ast::TypeBody::Has(members) = &declaration.body.value else { continue };
        for member in members {
            if let ast::TypeMember::Binding(binding) = &member.value {
                let span = source.span_table.get(member.span_id);
                let written = text.get(span.start()..span.end()).unwrap_or_default();
                // The header is what the method promises; a body is a default.
                let header = written.split(':').next().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
                methods.insert(binding.name.value.clone(), (binding.value.is_some(), header));
            }
        }
    }
    Some(methods)
}

fn trait_change(verdict: &mut Verdict, old: &(String, Node), new: &(String, Node)) {
    let (Some(a), Some(b)) = (trait_methods(&old.0), trait_methods(&new.0)) else {
        verdict.say(Compatibility::Unknown, "trait declaration changed (unreadable)");
        return;
    };
    for (name, (_, header)) in &a {
        match b.get(name) {
            None => verdict.say(Compatibility::Breaking, format!("trait method `{name}` removed")),
            Some((_, other)) if other != header => verdict.say(Compatibility::Breaking, format!("trait method `{name}` changed")),
            _ => {}
        }
    }
    for (name, (default, _)) in &b {
        if !a.contains_key(name) {
            if *default {
                verdict.say(Compatibility::Compatible, format!("trait method `{name}` added with a default"));
            } else {
                verdict.say(Compatibility::Breaking, format!("trait method `{name}` added without a default"));
            }
        }
    }
    if verdict.reasons.is_empty() && old.1 != new.1 {
        verdict.say(Compatibility::Unknown, "trait names changed");
    }
}

#[cfg(test)]
mod tests;
