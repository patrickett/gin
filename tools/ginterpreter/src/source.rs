use std::collections::{HashMap, HashSet};

use crate::runtime::{GenerationId, runtime_symbol_name};
use ast::ParamConvention;
use derive_more::From;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, From)]
pub struct SymbolSlot(u64);

impl SymbolSlot {
    pub fn as_u64(self) -> u64 {
        self.0
    }
}

pub struct SessionSource {
    declarations: Vec<SessionDeclaration>,
    declarations_text: String,
    runtime_declarations: HashMap<String, RuntimeDeclaration>,
    next_slot: u64,
    symbol_slots: HashMap<String, SymbolSlot>,
    evaluation_type: String,
}

impl Default for SessionSource {
    fn default() -> Self {
        Self {
            declarations: Vec::new(),
            declarations_text: String::new(),
            runtime_declarations: HashMap::new(),
            next_slot: 0,
            symbol_slots: HashMap::new(),
            evaluation_type: "Signed64".to_string(),
        }
    }
}

#[derive(Clone)]
struct SessionDeclaration {
    source: String,
    names: Vec<String>,
}

#[derive(Debug)]
pub struct GeneratedEvaluation {
    source: String,
    symbol: String,
    expression: String,
    expression_range: std::ops::Range<usize>,
}

#[derive(Debug, Clone)]
pub struct RuntimeDeclaration {
    pub(crate) name: String,
    signature: String,
    param_names: Vec<String>,
    param_conventions: Vec<ParamConvention>,
    is_constant: bool,
}

impl RuntimeDeclaration {
    pub fn new(
        name: String,
        signature: String,
        param_names: Vec<String>,
        param_conventions: Vec<ParamConvention>,
        is_constant: bool,
    ) -> Self {
        Self {
            name,
            signature,
            param_names,
            param_conventions,
            is_constant,
        }
    }

    fn declaration_source(&self, generation_symbol: &str, forward_to: &str) -> String {
        let suffix = self
            .signature
            .strip_prefix(self.name.as_str())
            .unwrap_or("")
            .to_string();
        let mut declaration = String::new();
        declaration.push_str(generation_symbol);
        declaration.push_str(&suffix);

        if self.is_constant {
            declaration.push_str(" := ");
            declaration.push_str(forward_to);
            declaration.push('\n');
            return declaration;
        }

        let mut locals = String::new();
        let mut args = Vec::new();
        for (index, name) in self.param_names.iter().enumerate() {
            let convention = self
                .param_conventions
                .get(index)
                .copied()
                .unwrap_or(ParamConvention::Own);
            match convention {
                ParamConvention::Own => {
                    let local_name = format!("__gin_dispatch_arg_{index}_{name}");
                    locals.push_str("    ");
                    locals.push_str(&local_name);
                    locals.push_str(" := eat ");
                    locals.push_str(name);
                    locals.push('\n');
                    args.push(local_name);
                }
                ParamConvention::Consume => args.push(format!("eat {name}")),
                ParamConvention::Observe => args.push(format!("ref {name}")),
                ParamConvention::Mutate => args.push(format!("mut {name}")),
            }
        }

        declaration.push_str(": ");
        if locals.is_empty() {
            declaration.push_str(forward_to);
            if args.is_empty() {
                declaration.push('\n');
            } else {
                declaration.push('(');
                declaration.push_str(&args.join(", "));
                declaration.push_str(")\n");
            }
            return declaration;
        }

        declaration.push('\n');
        declaration.push_str(&locals);
        declaration.push_str("    ");
        declaration.push_str("return ");
        declaration.push_str(forward_to);
        declaration.push('(');
        declaration.push_str(&args.join(", "));
        declaration.push_str(")\n");
        declaration
    }
}

impl SessionSource {
    pub fn with_evaluation_type(mut self, evaluation_type: impl Into<String>) -> Self {
        self.evaluation_type = evaluation_type.into();
        self
    }

    pub fn declarations(&self) -> &str {
        &self.declarations_text
    }

    pub fn symbol_slots(&self) -> &HashMap<String, SymbolSlot> {
        &self.symbol_slots
    }

    pub fn symbol_slot(&self, symbol: &str) -> Option<SymbolSlot> {
        self.symbol_slots.get(symbol).copied()
    }

    pub fn candidate(&self, submission: &str) -> String {
        let mut candidate =
            String::with_capacity(self.declarations_text.len() + submission.len() + 1);
        candidate.push_str(&self.declarations_text);
        candidate.push_str(submission);
        candidate.push('\n');
        candidate
    }

    pub fn candidate_without_shadowed(
        &self,
        submission: &str,
        declaration_names: &[String],
    ) -> String {
        if declaration_names.is_empty() {
            return self.candidate(submission);
        }

        let shadowed: HashSet<&str> = declaration_names.iter().map(|name| name.as_str()).collect();
        let mut candidate =
            String::with_capacity(self.declarations_text.len() + submission.len() + 1);
        for declaration in &self.declarations {
            if declaration
                .names
                .iter()
                .any(|name| shadowed.contains(name.as_str()))
            {
                continue;
            }
            candidate.push_str(&declaration.source);
        }
        candidate.push_str(submission);
        candidate.push('\n');
        candidate
    }

    pub fn accept_declaration(&mut self, submission: &str, declaration_names: Vec<String>) {
        self.accept_declaration_with_dispatches(submission, declaration_names, Vec::new())
    }

    pub fn accept_declaration_with_dispatches(
        &mut self,
        submission: &str,
        declaration_names: Vec<String>,
        runtime_declarations: Vec<RuntimeDeclaration>,
    ) {
        let declaration_names = dedupe_names(declaration_names);
        let shadowed: HashSet<&str> = declaration_names.iter().map(|name| name.as_str()).collect();
        for name in &declaration_names {
            self.symbol_slots.entry(name.clone()).or_insert_with(|| {
                let slot = SymbolSlot(self.next_slot);
                self.next_slot += 1;
                slot
            });
        }

        self.declarations.retain(|declaration| {
            !declaration
                .names
                .iter()
                .any(|name| shadowed.contains(name.as_str()))
        });
        for name in &declaration_names {
            self.runtime_declarations.remove(name);
        }

        self.declarations.push(SessionDeclaration {
            source: format!("{submission}\n"),
            names: declaration_names,
        });

        for declaration in runtime_declarations {
            self.runtime_declarations
                .insert(declaration.name.clone(), declaration);
        }

        let mut declarations = String::new();
        for declaration in &self.declarations {
            declarations.push_str(&declaration.source);
        }
        self.declarations_text = declarations;
    }

    pub fn clear(&mut self) {
        self.declarations.clear();
        self.declarations_text.clear();
        self.runtime_declarations.clear();
        self.symbol_slots.clear();
        self.next_slot = 0;
    }

    pub fn evaluation_candidate(&self, submission: &str, generation: u64) -> GeneratedEvaluation {
        let symbol = format!("__gin_interpreter_eval_{generation}");
        let prefix = format!("{symbol}() {}: ", self.evaluation_type);
        let expression_start = self.declarations_text.len() + prefix.len();
        let mut source = String::with_capacity(expression_start + submission.len() + 1);
        source.push_str(&self.declarations_text);
        source.push_str(&prefix);
        source.push_str(submission);
        source.push('\n');
        GeneratedEvaluation::new(
            source,
            symbol,
            submission.to_string(),
            expression_start..expression_start + submission.len(),
        )
    }

    pub fn evaluation_candidate_with_dispatches(
        &self,
        submission: &str,
        generation: u64,
        generations: &HashMap<String, GenerationId>,
    ) -> GeneratedEvaluation {
        let symbol = format!("__gin_interpreter_eval_{generation}");
        let runtime_dispatches = self.runtime_dispatch_source(generations);
        let prefix = format!("{symbol}() {}: ", self.evaluation_type);
        let expression_start =
            self.declarations_text.len() + runtime_dispatches.len() + prefix.len();
        let mut source = String::with_capacity(expression_start + submission.len() + 1);
        source.push_str(&self.declarations_text);
        source.push_str(&runtime_dispatches);
        source.push_str(&prefix);
        source.push_str(submission);
        source.push('\n');
        GeneratedEvaluation::new(
            source,
            symbol,
            submission.to_string(),
            expression_start..expression_start + submission.len(),
        )
    }

    pub(crate) fn runtime_dispatch_source(
        &self,
        generations: &HashMap<String, GenerationId>,
    ) -> String {
        let mut source = String::new();
        let mut emitted = HashSet::new();

        for declaration in &self.declarations {
            for name in &declaration.names {
                if !emitted.insert(name.clone()) {
                    continue;
                }

                let Some(generation) = generations.get(name) else {
                    continue;
                };
                let Some(declaration) = self.runtime_declarations.get(name) else {
                    continue;
                };
                let runtime_symbol = runtime_symbol_name(name, *generation);

                source.push_str(&declaration.declaration_source(&runtime_symbol, name));
            }
        }

        source
    }
}

impl GeneratedEvaluation {
    pub(crate) fn new(
        source: String,
        symbol: String,
        expression: String,
        expression_range: std::ops::Range<usize>,
    ) -> Self {
        Self {
            source,
            symbol,
            expression,
            expression_range,
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    pub fn expression(&self) -> &str {
        &self.expression
    }

    pub fn expression_range(&self) -> std::ops::Range<usize> {
        self.expression_range.clone()
    }
}

fn dedupe_names(mut declaration_names: Vec<String>) -> Vec<String> {
    declaration_names.sort();
    declaration_names.dedup();
    declaration_names
}
#[cfg(test)]
#[path = "tests/source_tests.rs"]
mod tests;
