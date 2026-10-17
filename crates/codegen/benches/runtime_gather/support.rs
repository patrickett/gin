pub const INDEX_CAPACITY: usize = 10_000;
pub const VALUE_CAPACITY: usize = 65_535;
pub const SOURCE: &str = include_str!("../../../test-fixtures/fixtures/runtime_gather_plan.gin");

pub struct Input {
    indices: Vec<u16>,
}

impl Input {
    pub fn new(indices: &[u16]) -> Self {
        Self {
            indices: indices.to_vec(),
        }
    }

    pub fn indices(&self) -> &[u16] {
        &self.indices
    }

    pub fn indices_mut(&mut self) -> &mut [u16] {
        &mut self.indices
    }
}

pub struct Plan {
    indices: Vec<u16>,
    extents: Vec<u16>,
}

impl Plan {
    pub fn indices(&self) -> &[u16] {
        &self.indices
    }

    pub fn extents(&self) -> Vec<u16> {
        self.extents.clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    CheckedCompact,
    CheckedWitness,
    Prepared,
}

pub struct Kernels;

impl Kernels {
    pub const fn new() -> Self {
        Self
    }

    pub fn prepare(&self, input: &Input, extent: u16) -> Option<Plan> {
        if input.indices.len() > INDEX_CAPACITY || usize::from(extent) > VALUE_CAPACITY {
            return None;
        }
        if input.indices.iter().any(|&index| index >= extent) {
            return None;
        }
        Some(Plan {
            indices: input.indices.clone(),
            extents: vec![extent; input.indices.len()],
        })
    }

    pub fn selected_sum(
        &self,
        mode: Mode,
    ) -> impl Fn(&Plan, &Input, &[u8; VALUE_CAPACITY]) -> u64 + '_ {
        move |plan, input, values| match mode {
            Mode::CheckedCompact => input
                .indices
                .iter()
                .map(|&index| values.get(usize::from(index)).copied().map_or(0, u64::from))
                .sum(),
            Mode::CheckedWitness => plan
                .indices
                .iter()
                .zip(&plan.extents)
                .map(|(&index, &extent)| {
                    if index < extent {
                        values.get(usize::from(index)).copied().map_or(0, u64::from)
                    } else {
                        0
                    }
                })
                .sum(),
            Mode::Prepared => plan
                .indices
                .iter()
                .map(|&index| u64::from(values[usize::from(index)]))
                .sum(),
        }
    }
}
