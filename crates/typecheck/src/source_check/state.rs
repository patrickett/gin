//! State ownership and reset boundaries. Accounting and source stay on `Checker`:
//! no proof, validation, or replay transition refunds work or flow.
use super::*;

/// Persistent within one module check, including all bodies and replayed worlds.
#[derive(Default)]
pub(super) struct ModuleState<'a> {
    pub(super) schemas: Vec<&'a TypeDeclaration>,
    /// Transparent type aliases, local and imported. See `aliases.rs`.
    pub(super) aliases: Vec<aliases::Alias<'a>>,
    /// Parallel to `schemas`: the capability the declaration asked for.
    pub(super) schema_capability: Vec<Capability>,
    /// Parallel to `schemas`: the `has` block paired with a linear domain of the same name.
    /// Parallel to `schemas`: whether `#wrap` was written on a domain.
    pub(super) schema_wrap: Vec<bool>,
    pub(super) schema_domain_has: Vec<Option<&'a TypeDeclaration>>,
    /// While declaring methods: the next schema in source order, so a `has` block finds
    /// its own schema without searching.
    pub(super) schema_cursor: usize,
    /// Trait declarations: a `has` block of methods with no fields, which no value has.
    pub(super) traits: Vec<(&'a TypeDeclaration, SpanId)>,
    pub(super) instances: Vec<aggregates::Instance>,
    /// Sorted handles only: arena IDs remain append-only and topological.
    pub(super) instance_index: Vec<usize>,
    pub(super) layouts: Vec<contracts::Layout>,
    pub(super) type_names: Vec<(String, usize)>,
    pub(super) function_names: Vec<(String, usize)>,
    /// Module constants (`name := expression`), local and imported. See `constants.rs`.
    pub(super) constants: constants::Constants<'a>,
    /// Source-free structural substitution, keyed by the complete type environment.
    pub(super) substitutions: Vec<((SourceType, Vec<SourceType>), SourceType)>,
    pub(super) functions: Vec<Option<Rc<calls::Function>>>,
    /// Local `#infix` methods by operator and function index, known before any body is
    /// checked so the call graph can order them ahead of their uses.
    pub(super) infix_decls: Vec<(InfixOperator, usize)>,
    /// (function index, trait name) for each method a provided trait added to a type.
    pub(super) provided_from: Vec<(usize, String)>,
    /// Advisory ordering edges per function: method and operator uses whose receiver type
    /// is not known to the call graph. See `function_edges`.
    pub(super) weak_edges: Vec<Vec<usize>>,
    /// Non-fatal findings, copied into the checked file when the module finishes.
    pub(super) notes: Vec<SourceCheckNote>,
    /// Origin of each imported function, aligned with the tail of `functions`:
    /// entry `i` is table index `functions.len() - imported.len() + i`.
    pub(super) imported: Vec<(u32, u32)>,
    /// Functions marked `#instruction`, with their mnemonics.
    pub(super) instructions: Vec<(&'a Binding, String)>,
    /// Functions marked `#load` or `#store`.
    pub(super) memory: Vec<(&'a Binding, MemoryOp)>,
    /// Variants a union declared itself: (union schema, bare name, layout). Not in
    /// `type_names`: a variant is found through its union.
    pub(super) union_variants: Vec<(usize, &'a str, usize)>,
    /// The payload type written in `Variant(Type)` for a variant a union declares, by layout.
    pub(super) variant_payloads: Vec<(usize, Vec<(&'a str, &'a Expression)>)>,
    /// The names of the payload fields of the variants unions and functions declared, by layout,
    /// from the source: what a claim may refer to. Empty strings are unnamed payloads.
    pub(super) variant_field_names: Vec<(usize, Vec<&'a str>)>,
    /// The records a function that returns several named values returns, by binding address:
    /// their layouts, which are built the first time the function is checked.
    pub(super) anonymous_records: Vec<(usize, usize)>,
    /// The sum a function with inline result variants returns, by binding address: its type and
    /// its variants, built the first time the function's header is read.
    pub(super) hidden_results: Vec<(usize, SourceType, Vec<SourceType>)>,
    /// The layouts of the variants of those sums: their first payload may be given unlabeled.
    pub(super) hidden_variants: Vec<usize>,
    /// The function marked by `#entry`, as an index into `functions`.
    pub(super) entry: Option<usize>,
    /// This module's nominal origin, shared by every layout it makes (declared or not), even
    /// when it declares no type of its own. Fresh per check.
    pub(super) origin: std::sync::Arc<()>,
    /// The qualified name of the function whose body is being checked, for the stable key of
    /// a layout the checker makes inside it.
    pub(super) current_function: String,
}

/// The types the checker settled for one call or construction of a generic body: the type
/// arguments the callee was applied to (callee's parameters in order, a generic type's own
/// first), and the type of the value it yields.
#[derive(Clone)]
pub(super) struct CallInfo {
    pub(super) types: Vec<SourceType>,
    pub(super) result: SourceType,
}

/// Proof workspace used for declaration validation and each verified body world.
/// Clearing vectors at boundaries retains the existing allocation reuse policy.
#[derive(Default)]
pub(super) struct ProofState<'a> {
    pub(super) names: Vec<(&'a str, Value)>,
    /// Names in scope that `::` may reassign: `mut` parameters and `:` locals. Rebinding a
    /// name through `insert` clears any stale entry, so scope exit cannot leak mutability.
    pub(super) assignable: Vec<&'a str>,
    pub(super) generics: Vec<&'a str>,
    pub(super) owners: Vec<aggregates::Owner>,
    /// Whether a handle has taken a loan in this body, so that loan checks can be skipped.
    pub(super) loaned: bool,
    pub(super) facts: Vec<(u32, BinaryOperator, u32)>,
    /// `result = left +- right` equations between identities (`linear.rs`).
    pub(super) definitions: Vec<linear::Definition>,
    /// `(generic index, is alignment, identity)`: the number `#size(T)` / `#alignment(T)` of a type
    /// parameter, one identity per query, so two readings of it are the same number.
    pub(super) layout_ids: Vec<(u32, bool, u32)>,
    /// Assuming an arithmetic claim: relate its sums even when nothing is related yet.
    pub(super) relate_always: bool,
    /// Applying a callee's effects: stores assume the record's claims instead of proving them.
    pub(super) trusting_effects: bool,
    pub(super) scalars: Vec<(u32, I256, I256)>,
    pub(super) integer_guards: Vec<(u32, I256)>,
    /// `(identity, modulus)`: the number is known to be a multiple of the modulus.
    pub(super) multiples: Vec<(u32, I256)>,
    pub(super) next_identity: u32,
    pub(super) parameter_roles: Vec<calls::Parameter>,
    /// Names of an instruction's unannotated parameters, in order.
    pub(super) literals: Vec<&'a str>,
    /// Parallel to `parameter_roles`.
    pub(super) parameter_passing: Vec<calls::Passing>,
    /// Names of parameters that only add a bound to a type parameter of the receiver's type:
    /// written like parameters, but no role and no argument.
    pub(super) added_bounds: Vec<&'a str>,
    /// The receiver type of the method being checked, which types its bare `self`.
    pub(super) receiver: Option<SourceType>,
    /// The schema whose `has` block declares the method being checked.
    pub(super) receiver_schema: Option<usize>,
    /// The trait whose bodiless method is being given its signature.
    pub(super) abstract_trait: Option<&'a TypeDeclaration>,
    /// Bounds on the type parameters: the generic's index and the trait expression it must provide.
    pub(super) bounds: Vec<(u32, &'a Expression)>,
    /// The type parameters that are compile-time numbers (`capacity Length`). See `numbers.rs`.
    pub(super) numbers: Vec<numbers::NumberParam>,
    /// The method being checked is a type's own `copy`, where `deref self` may duplicate a
    /// linear value.
    pub(super) in_copy: bool,
    /// Type arguments a call through a bound supplies, in place of the receiver's.
    pub(super) forced_types: Option<Vec<SourceType>>,
    /// The type a call is expected to yield, for the type arguments its arguments leave open
    /// (`inference.rs`). Set by `evaluate_call`, taken by `call_tasks`.
    pub(super) call_expected: Option<SourceType>,
    /// What the checker decided at each call or construction made inside a generic body, by the
    /// expression's span, so that recording a template body does not resolve types again. Kept
    /// across bodies (spans are unique, and a body checked again may not repeat its calls).
    pub(super) call_info: Vec<(SpanId, CallInfo)>,
    /// The operator the method being checked implements through `#infix`.
    pub(super) infix: Option<InfixOperator>,
    /// The method being checked is a trait default, so its result type is inferred from
    /// the value it returns rather than written (the type is not known to the trait).
    pub(super) infer_result: bool,
    /// The result a provided trait gives this method: the argument written for the trait's
    /// result parameter (`Add(IntegerResult)`), against which the method's result is checked.
    pub(super) trait_result: Option<&'a Expression>,
    pub(super) summary: Option<summaries::Summary>,
    /// The `where` clauses of the function being verified, with their values at entry.
    pub(super) clauses: Vec<postconditions::Clause<'a>>,
    /// For a result written as variants: the `::` transitions of each variant, by position.
    pub(super) variant_clauses: Vec<Vec<postconditions::Clause<'a>>>,
    /// The position of the variant this world returned, among the written ones.
    pub(super) exit_variant: Option<usize>,
    /// The transitions a caller learns where each member is matched.
    pub(super) variant_effects: Vec<(SourceType, Vec<(postconditions::PlacePath, postconditions::ClauseExpr)>)>,
    /// The function's `where` claims over its parameters and its result, as constraints a
    /// caller assumes after the call (the result is the slot after the parameters).
    pub(super) postconditions: Vec<contracts::Constraint>,
    /// The same for a result written as variants: the claims of each member, by member type.
    pub(super) variant_postconditions: Vec<(SourceType, Vec<contracts::Constraint>)>,
    /// A body-less function whose result is written as variants: one summary per member, so a
    /// call is in a world where it is that member.
    pub(super) member_summaries: Vec<summaries::Summary>,
    /// The members a result written as variants is the sum of, in written order; empty for a
    /// result that is one type.
    pub(super) variants: Vec<SourceType>,
    /// The `mut` parameter places the body may have changed.
    pub(super) writes: Vec<postconditions::PlacePath>,
    /// What the function does to its `mut` parameters, as callers will apply it.
    pub(super) effects: Option<Vec<postconditions::Effect>>,
    /// How many loop bodies the evaluation is inside. A `return` there ends the whole path.
    pub(super) loop_depth: u32,
    /// A `return` ran inside a loop body: the evaluations around it unwind to its value.
    pub(super) returned: bool,
    /// The value that `return` produced, carried up through the loops it ended.
    pub(super) returned_value: Option<Value>,
    /// The declared result of the function being checked, which a `return` inside a loop is
    /// typed against (the loop's own expected type is its unit).
    pub(super) result_expected: Option<SourceType>,
    /// The function being checked is written `ref T` / `mut T`: its result is a borrow.
    pub(super) result_borrow: Option<BorrowKind>,
    /// The parameters a returned value borrows from, gathered at each `return`.
    pub(super) result_sources: Vec<borrowing_results::ResultSource>,
}

impl ProofState<'_> {
    pub(super) fn begin_body(&mut self) {
        self.names.clear();
        self.assignable.clear();
        self.generics.clear();
        self.bounds.clear();
        self.numbers.clear();
        self.parameter_roles.clear();
        self.literals.clear();
        self.parameter_passing.clear();
        self.added_bounds.clear();
        self.owners.clear();
        self.loaned = false;
        self.facts.clear();
        self.definitions.clear();
        self.layout_ids.clear();
        self.scalars.clear();
        self.integer_guards.clear();
        self.multiples.clear();
        self.clauses.clear();
        self.variant_clauses.clear();
        self.exit_variant = None;
        self.variant_effects.clear();
        self.postconditions.clear();
        self.variant_postconditions.clear();
        self.member_summaries.clear();
        self.variants.clear();
        self.writes.clear();
        self.effects = None;
        self.loop_depth = 0;
        self.returned = false;
        self.returned_value = None;
        self.result_expected = None;
        self.result_borrow = None;
        self.result_sources.clear();
        self.next_identity = 0;
    }

    /// Layout checking has no body summary. In particular it does not reset the
    /// identity counter or the other proof fields as body entry does.
    pub(super) fn finish_layout_validation(&mut self) {
        self.owners.clear();
        self.facts.clear();
        self.definitions.clear();
        self.layout_ids.clear();
        self.scalars.clear();
    }
}

/// Choices belong to a world traversal; immutable-source caches span its worlds,
/// but are allocated anew for the next function. Scalar domains are not cached.
#[derive(Default)]
pub(super) struct ReplayState {
    pub(super) choices: Vec<(usize, usize)>,
    /// Parallel to `choices`: the choice picks which of a callee's outcomes to apply (not a
    /// branch of this function's own code).
    pub(super) outcome_flags: Vec<bool>,
    pub(super) choice_cursor: usize,
    /// Keys are source nodes/strings, never SpanId.
    pub(super) symbol_cache: Option<SymbolCache>,
    pub(super) type_cache: Option<Vec<((usize, bool), SourceType)>>,
}

impl ReplayState {
    pub(super) fn begin_function(&mut self) {
        assert!(self.choices.is_empty());
        self.symbol_cache = Some(Vec::new());
        self.type_cache = Some(Vec::new());
    }

    pub(super) fn begin_world(&mut self) {
        self.choice_cursor = 0;
    }

    pub(super) fn finish_function(&mut self) {
        self.choice_cursor = 0;
        self.symbol_cache = None;
        self.type_cache = None;
    }

    pub(super) fn finish_layout_validation(&mut self) {
        self.choices.clear();
        self.outcome_flags.clear();
        self.choice_cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn populated(source: &SourceFile) -> Checker<'_> {
        let mut checker = Checker::new(source, crate::Limits::default());
        checker.proof.generics.push("element");
        checker.proof.parameter_roles.push(calls::Parameter::Type);
        let template = checker
            .array_type(SourceType::Generic { index: 0 }, 4)
            .unwrap();
        let array = checker.input_value(template).unwrap();
        checker.array_child(array, 2).unwrap();
        checker.insert("values", array).unwrap();
        let scalar = checker
            .fresh_value(Value::integer(I256::ZERO, I256::from(2)).ty)
            .unwrap();
        checker.insert("number", scalar).unwrap();
        checker.guard_integer(scalar, I256::ONE).unwrap();
        let identity = scalar.identity.unwrap();
        checker
            .proof
            .facts
            .push((identity, BinaryOperator::Equal, identity));
        checker.substitute(template, &[scalar.ty]).unwrap();
        let inputs = checker.summary_inputs().unwrap();
        checker.proof.summary = Some(checker.summarize(&inputs, Some(array)).unwrap());
        checker
    }

    fn capacities(proof: &ProofState<'_>) -> [usize; 7] {
        [
            proof.names.capacity(),
            proof.generics.capacity(),
            proof.parameter_roles.capacity(),
            proof.owners.capacity(),
            proof.facts.capacity(),
            proof.scalars.capacity(),
            proof.integer_guards.capacity(),
        ]
    }

    #[test]
    fn body_reset_reuses_storage_without_resetting_module_session_or_summary() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = populated(&source);
        let capacities_before = capacities(&checker.proof);
        assert!(capacities_before.iter().all(|capacity| *capacity > 0));
        let summary = checker.proof.summary.clone();
        let instances = checker.module.instances.as_ptr();
        let instance_count = checker.module.instances.len();
        let index = checker.module.instance_index.clone();
        let substitutions = checker.module.substitutions.clone();
        let accounting = (checker.remaining, checker.flow_remaining);
        checker.replay.begin_function();
        checker.replay.choices.push((1, 2));
        checker.replay.choice_cursor = 1;

        checker.proof.begin_body();

        assert!(checker.proof.names.is_empty());
        assert!(checker.proof.generics.is_empty());
        assert!(checker.proof.parameter_roles.is_empty());
        assert!(checker.proof.owners.is_empty());
        assert!(checker.proof.facts.is_empty());
        assert!(checker.proof.scalars.is_empty());
        assert!(checker.proof.integer_guards.is_empty());
        assert_eq!(checker.proof.next_identity, 0);
        // The caller asserts summary absence; resetting must not silently discard one.
        assert_eq!(checker.proof.summary, summary);
        assert_eq!(capacities(&checker.proof), capacities_before);
        assert_eq!(checker.module.instances.as_ptr(), instances);
        assert_eq!(checker.module.instances.len(), instance_count);
        assert_eq!(checker.module.instance_index, index);
        assert_eq!(checker.module.substitutions, substitutions);
        assert_eq!((checker.remaining, checker.flow_remaining), accounting);
        assert_eq!(checker.replay.choices, [(1, 2)]);
        assert_eq!(checker.replay.choice_cursor, 1);
        assert!(checker.replay.symbol_cache.is_some());
        assert!(checker.replay.type_cache.is_some());
    }

    #[test]
    fn layout_cleanup_is_narrower_than_body_reset() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = populated(&source);
        let capacities_before = capacities(&checker.proof);
        let identity = checker.proof.next_identity;
        let summary = checker.proof.summary.clone();
        let accounting = (checker.remaining, checker.flow_remaining);
        checker.replay.begin_function();
        checker
            .replay
            .symbol_cache
            .as_mut()
            .unwrap()
            .push(((1, 2, false), None));
        checker
            .replay
            .type_cache
            .as_mut()
            .unwrap()
            .push(((3, false), SourceType::Unit));
        checker.replay.choices.push((1, 2));
        checker.replay.choice_cursor = 1;

        checker.proof.finish_layout_validation();
        checker.replay.finish_layout_validation();

        assert!(checker.proof.owners.is_empty());
        assert!(checker.proof.facts.is_empty());
        assert!(checker.proof.scalars.is_empty());
        assert_eq!(checker.proof.names.len(), 2);
        assert_eq!(checker.proof.generics, ["element"]);
        assert_eq!(checker.proof.parameter_roles.len(), 1);
        assert_eq!(checker.proof.integer_guards.len(), 1);
        assert_eq!(checker.proof.next_identity, identity);
        assert_eq!(checker.proof.summary, summary);
        assert_eq!(capacities(&checker.proof), capacities_before);
        assert!(checker.replay.choices.is_empty());
        assert_eq!(checker.replay.choice_cursor, 0);
        assert_eq!(checker.replay.symbol_cache.as_ref().unwrap().len(), 1);
        assert_eq!(checker.replay.type_cache.as_ref().unwrap().len(), 1);
        assert_eq!((checker.remaining, checker.flow_remaining), accounting);
    }

    #[test]
    fn replay_caches_span_worlds_but_not_functions() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        checker.replay.begin_function();
        checker
            .cache_source_type((7, false), SourceType::Unit)
            .unwrap();
        checker.lookup_symbol("missing", false).unwrap();
        let symbols = checker.replay.symbol_cache.as_ref().unwrap().as_ptr();
        let types = checker.replay.type_cache.as_ref().unwrap().as_ptr();
        checker.replay.choices.push((1, 2));
        checker.replay.choice_cursor = 1;
        let accounting = (checker.remaining, checker.flow_remaining);

        checker.replay.begin_world();
        checker.proof.begin_body();

        assert_eq!(checker.replay.choice_cursor, 0);
        assert_eq!(checker.replay.choices, [(1, 2)]);
        assert_eq!(
            checker.replay.symbol_cache.as_ref().unwrap().as_ptr(),
            symbols
        );
        assert_eq!(checker.replay.type_cache.as_ref().unwrap().as_ptr(), types);
        assert_eq!(checker.replay.symbol_cache.as_ref().unwrap().len(), 1);
        assert_eq!(checker.replay.type_cache.as_ref().unwrap().len(), 1);
        assert_eq!((checker.remaining, checker.flow_remaining), accounting);
        checker.replay.choices.pop();
        checker.replay.finish_function();
        assert!(checker.replay.symbol_cache.is_none());
        assert!(checker.replay.type_cache.is_none());
        assert_eq!(checker.replay.choice_cursor, 0);
        checker.replay.begin_function();
        assert!(checker.replay.symbol_cache.as_ref().unwrap().is_empty());
        assert!(checker.replay.type_cache.as_ref().unwrap().is_empty());
        assert_eq!((checker.remaining, checker.flow_remaining), accounting);
    }

    #[test]
    fn layout_proofs_and_body_proofs_share_module_types_not_identities_or_names() {
        let source = parser::parse_strict(concat!(
            "Box(element) has value element, number in 0 to 2\n",
            "first(element Type, input Box(element)) Box(element):\n  local: input\n  return local\n",
            "second(other Type, output Box(other)) Box(other):\n  alias: output\n  return alias\n",
        ))
        .unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        checker.collect().unwrap();
        checker.prepare_layouts().unwrap();
        let schemas = checker.validate_source_schemas().unwrap();
        checker.compile_layouts(schemas);
        let before_layout = (checker.remaining, checker.flow_remaining);
        checker.validate_layouts().unwrap();
        assert!(checker.proof.next_identity > 0);
        assert!(checker.proof.owners.is_empty());
        assert!(checker.proof.facts.is_empty());
        assert!(checker.proof.scalars.is_empty());
        assert!(checker.remaining < before_layout.0);
        assert!(checker.flow_remaining < before_layout.1);
        let Item::Binding(first) = &source.items[1].value else {
            unreachable!()
        };
        let Item::Binding(second) = &source.items[2].value else {
            unreachable!()
        };
        let first = Rc::new(
            checker
                .function_worlds(first, checker.parameters_max)
                .unwrap(),
        );
        let instance_count = checker.module.instances.len();
        checker.module.functions.push(Some(first.clone()));
        let before_second = (checker.remaining, checker.flow_remaining);
        let second = checker
            .function_worlds(second, checker.parameters_max)
            .unwrap();
        assert_eq!(checker.proof.generics, ["other"]);
        assert_eq!(checker.proof.names.len(), 1);
        assert_eq!(checker.proof.names[0].0, "output");
        assert_eq!(checker.proof.names[0].1.identity, Some(0));
        assert_eq!(checker.proof.next_identity, 1);
        assert_eq!(checker.proof.owners.len(), 1);
        assert!(checker.proof.summary.is_none());
        assert!(checker.proof.parameter_roles.is_empty());
        assert!(checker.replay.choices.is_empty());
        assert!(checker.replay.symbol_cache.is_none());
        assert!(checker.replay.type_cache.is_none());
        assert_eq!(checker.module.instances.len(), instance_count);
        assert!(Rc::ptr_eq(
            checker.module.functions[0].as_ref().unwrap(),
            &first
        ));
        assert_eq!(first.summaries, second.summaries);
        assert_eq!(second.summaries[0].outputs[0].input, Some(0));
        assert!(checker.remaining < before_second.0);
        assert!(checker.flow_remaining < before_second.1);
    }
}
