pub const CAPACITY: usize = 1024;

pub struct Kernels;

pub struct Plan {
    length: u16,
    indices: [u16; 4],
}

impl Kernels {
    pub const fn new() -> Self {
        Self
    }

    pub fn prepare(&self, length: u16, indices: [u16; 4]) -> Option<Plan> {
        if length == 0 || usize::from(length) > CAPACITY {
            return None;
        }
        if indices.iter().any(|&index| index >= length) {
            return None;
        }
        Some(Plan { length, indices })
    }

    pub fn sum_prepared(&self, plan: &Plan, values: &[u8; CAPACITY]) -> u64 {
        plan.indices
            .iter()
            .map(|&index| u64::from(values[usize::from(index)]))
            .sum()
    }

    pub fn sum_checked(&self, plan: &Plan, values: &[u8; CAPACITY]) -> u64 {
        plan.indices
            .iter()
            .map(|&index| values.get(usize::from(index)).copied().map_or(0, u64::from))
            .sum()
    }

    pub fn selected_sum(&self, prepared: bool) -> impl Fn(&Plan, &[u8; CAPACITY]) -> u64 + '_ {
        move |plan, values| {
            if prepared {
                self.sum_prepared(plan, values)
            } else {
                self.sum_checked(plan, values)
            }
        }
    }
}

impl Plan {
    pub fn length(&self) -> u16 {
        self.length
    }

    pub fn indices(&self) -> [u16; 4] {
        self.indices
    }
}
