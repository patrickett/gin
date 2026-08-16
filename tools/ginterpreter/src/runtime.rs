use ast::ParamConvention;
use codegen::NativeEngine;
use derive_more::From;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStorage {
    Int64(i64),
    Bytes(Vec<u8>),
}

impl RuntimeStorage {
    pub fn zero(size: usize) -> Self {
        if size == 8 {
            Self::Int64(0)
        } else {
            Self::Bytes(vec![0_u8; size])
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeType {
    pub size: u8,
    pub align: u8,
}

impl RuntimeType {
    pub const I64: Self = Self { size: 8, align: 8 };

    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.size == other.size && self.align == other.align
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeDeclarationKind {
    Value,
    Function,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFunction {
    pub return_type: RuntimeType,
    pub parameter_types: Vec<RuntimeType>,
    pub parameter_conventions: Vec<ParamConvention>,
    /// Layout of any captured runtime environment.
    ///
    /// Capturing not yet supported, so all compatible redefinitions must share
    /// this empty layout.
    pub capture_layout: Vec<RuntimeType>,
}

impl RuntimeFunction {
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.return_type.is_compatible_with(&other.return_type)
            && self.parameter_types.len() == other.parameter_types.len()
            && self
                .parameter_types
                .iter()
                .zip(other.parameter_types.iter())
                .all(|(a, b)| a.is_compatible_with(b))
            && self
                .parameter_conventions
                .iter()
                .zip(other.parameter_conventions.iter())
                .all(|(a, b)| a == b)
            && self.capture_layout.len() == other.capture_layout.len()
            && self
                .capture_layout
                .iter()
                .zip(other.capture_layout.iter())
                .all(|(a, b)| a.is_compatible_with(b))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeValue {
    pub kind: RuntimeDeclarationKind,
    pub ty: RuntimeType,
    pub storage: RuntimeStorage,
    pub function: Option<RuntimeFunction>,
}

impl RuntimeValue {
    pub const fn new(ty: RuntimeType, storage: RuntimeStorage) -> Self {
        Self {
            kind: RuntimeDeclarationKind::Value,
            ty,
            storage,
            function: None,
        }
    }

    pub fn new_function(
        return_type: RuntimeType,
        parameter_types: Vec<RuntimeType>,
        parameter_conventions: Vec<ParamConvention>,
        capture_layout: Vec<RuntimeType>,
    ) -> Self {
        Self {
            kind: RuntimeDeclarationKind::Function,
            ty: return_type,
            storage: RuntimeStorage::Bytes(Vec::new()),
            function: Some(RuntimeFunction {
                return_type,
                parameter_types,
                parameter_conventions,
                capture_layout,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, From)]
pub struct GenerationId(pub u64);

#[derive(Debug)]
pub struct NativeExport {
    pub logical_symbol: String,
    pub generation_symbol: String,
}

pub struct NativeGeneration {
    pub id: GenerationId,
    pub engine: Option<NativeEngine>,
    pub exports: Vec<NativeExport>,
}

#[derive(Default)]
pub struct LiveRuntime {
    generations: Vec<NativeGeneration>,
    symbols: HashMap<String, GenerationId>,
    values: HashMap<String, RuntimeValue>,
    active_calls: HashMap<GenerationId, usize>,
}

impl LiveRuntime {
    pub fn generations(&self) -> &[NativeGeneration] {
        &self.generations
    }

    pub fn symbols(&self) -> &HashMap<String, GenerationId> {
        &self.symbols
    }

    pub fn generation_for_symbol(&self, symbol: &str) -> Option<GenerationId> {
        self.symbols.get(symbol).copied()
    }

    pub fn value_for_symbol(&self, symbol: &str) -> Option<&RuntimeValue> {
        self.values.get(symbol)
    }

    pub fn values(&self) -> &HashMap<String, RuntimeValue> {
        &self.values
    }

    pub fn generation(&self, generation_id: GenerationId) -> Option<&NativeGeneration> {
        self.generations
            .iter()
            .find(|generation| generation.id == generation_id)
    }

    pub fn track_active_generations<I>(&mut self, generation_ids: I) -> ActiveGenerations
    where
        I: IntoIterator<Item = GenerationId>,
    {
        let generation_ids: Vec<_> = generation_ids
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        for generation_id in generation_ids.iter() {
            *self.active_calls.entry(*generation_id).or_insert(0) += 1;
        }

        ActiveGenerations { generation_ids }
    }

    pub fn track_active_generations_for_symbols(
        &mut self,
        symbols: &HashMap<String, GenerationId>,
    ) -> ActiveGenerations {
        self.track_active_generations(symbols.values().copied())
    }

    pub fn finish_active_generations(
        &mut self,
        ActiveGenerations { generation_ids }: ActiveGenerations,
    ) {
        for generation_id in generation_ids.iter() {
            let Some(active_calls) = self.active_calls.get_mut(generation_id) else {
                continue;
            };
            if let Some(next) = active_calls.checked_sub(1) {
                if next == 0 {
                    self.active_calls.remove(generation_id);
                } else {
                    *active_calls = next;
                }
            }
        }
        self.retire_unused_generations();
    }

    pub fn store_value(&mut self, symbol: impl Into<String>, value: RuntimeValue) {
        self.values.insert(symbol.into(), value);
    }

    pub fn install_generation(&mut self, generation: NativeGeneration) {
        for export in &generation.exports {
            self.symbols
                .insert(export.logical_symbol.clone(), generation.id);
        }
        self.generations.push(generation);
    }

    pub fn clear(&mut self) {
        self.generations.clear();
        self.symbols.clear();
        self.values.clear();
        self.active_calls.clear();
    }

    pub fn retire_unused_generations(&mut self) {
        let symbol_generations: HashSet<_> = self.symbols.values().copied().collect();
        let mut retained = Vec::with_capacity(self.generations.len());
        for generation in self.generations.drain(..) {
            let active_calls = self.active_calls.get(&generation.id).copied().unwrap_or(0);
            if active_calls > 0 || symbol_generations.contains(&generation.id) {
                retained.push(generation);
            }
        }
        let retained_ids: HashSet<_> = retained.iter().map(|generation| generation.id).collect();
        self.active_calls
            .retain(|generation_id, _| retained_ids.contains(generation_id));
        self.generations = retained;
    }
}

#[derive(Debug, Clone)]
pub struct ActiveGenerations {
    generation_ids: Vec<GenerationId>,
}

pub fn runtime_symbol_name(logical_symbol: &str, generation: GenerationId) -> String {
    format!("__gin_live_{logical_symbol}_generation_{}", generation.0)
}

pub fn generation_wrapper_name(generation: GenerationId) -> String {
    format!("__gin_live_generation_{}", generation.0)
}
#[cfg(test)]
#[path = "tests/runtime_tests.rs"]
mod tests;
