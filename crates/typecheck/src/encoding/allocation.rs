use internment::Intern;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Register {
    pub name: Intern<String>,
    pub code: u8,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RegisterSlot {
    pub name: Intern<String>,
    pub candidates: Vec<Register>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Allocation {
    pub registers: Vec<Register>,
}

impl Allocation {
    pub(super) fn assign(
        slots: &[RegisterSlot],
        clobbers: &[Intern<String>],
    ) -> Result<Self, String> {
        if slots.len() > 16 {
            return Err("register allocation supports at most 16 slots".into());
        }
        let mut physical = Vec::<Register>::new();
        let mut choices = Vec::with_capacity(slots.len());
        for (index, slot) in slots.iter().enumerate() {
            if slot.candidates.len() > 16 {
                return Err("register slot has more than 16 candidates".into());
            }
            if slots[..index].iter().any(|other| other.name == slot.name) {
                return Err("duplicate register slot".into());
            }
            if slot.candidates.is_empty() {
                return Err("register slot has no candidates".into());
            }
            let mut row: Vec<usize> = Vec::with_capacity(slot.candidates.len());
            for candidate in &slot.candidates {
                if row.iter().any(|&i| physical[i].name == candidate.name) {
                    return Err("register slot repeats a candidate".into());
                }
                let physical_index =
                    if let Some(i) = physical.iter().position(|r| r.name == candidate.name) {
                        if physical[i].code != candidate.code {
                            return Err("register has conflicting codes".into());
                        }
                        i
                    } else {
                        physical.push(candidate.clone());
                        physical.len() - 1
                    };
                row.push(physical_index);
            }
            row.retain(|&index| !clobbers.contains(&physical[index].name));
            choices.push(row);
        }
        let mut owners = vec![None; physical.len()];
        for slot in 0..slots.len() {
            if !Self::augment(
                slot,
                &choices,
                &mut owners,
                &mut vec![false; physical.len()],
            ) {
                return Err("register allocation is unsatisfiable".into());
            }
        }
        let mut registers = vec![None; slots.len()];
        for (register, owner) in owners.into_iter().enumerate() {
            if let Some(slot) = owner {
                registers[slot] = Some(physical[register].clone());
            }
        }
        Ok(Self {
            registers: registers.into_iter().map(Option::unwrap).collect(),
        })
    }

    fn augment(
        slot: usize,
        choices: &[Vec<usize>],
        owners: &mut [Option<usize>],
        seen: &mut [bool],
    ) -> bool {
        for &register in &choices[slot] {
            if seen[register] {
                continue;
            }
            seen[register] = true;
            if owners[register].is_none()
                || Self::augment(owners[register].unwrap(), choices, owners, seen)
            {
                owners[register] = Some(slot);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn name(value: &str) -> Intern<String> {
        Intern::new(value.to_owned())
    }
    fn register(value: &str, code: u8) -> Register {
        Register {
            name: name(value),
            code,
        }
    }
    fn slot(value: &str, candidates: Vec<Register>) -> RegisterSlot {
        RegisterSlot {
            name: name(value),
            candidates,
        }
    }

    #[test]
    fn constrained_backtrack() {
        let result = Allocation::assign(
            &[
                slot("a", vec![register("rax", 0), register("rcx", 1)]),
                slot("b", vec![register("rax", 0)]),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(
            result.registers,
            vec![register("rcx", 1), register("rax", 0)]
        );
    }
    #[test]
    fn unsatisfiable_large_hall_set() {
        let candidates = (0..15)
            .map(|i| register(&format!("r{i}"), i))
            .collect::<Vec<_>>();
        let slots = (0..16)
            .map(|i| slot(&format!("s{i}"), candidates.clone()))
            .collect::<Vec<_>>();
        assert!(
            Allocation::assign(&slots, &[])
                .unwrap_err()
                .contains("unsatisfiable")
        );
    }
    #[test]
    fn validates_clobbers_and_duplicates() {
        let allocation = Allocation::assign(
            &[slot("a", vec![register("rax", 0), register("rcx", 1)])],
            &[name("rax")],
        )
        .unwrap();
        assert_eq!(allocation.registers, vec![register("rcx", 1)]);
        assert!(
            Allocation::assign(&[slot("a", vec![register("rax", 0)])], &[name("rax")]).is_err()
        );
        assert!(Allocation::assign(&[slot("a", vec![])], &[]).is_err());
        assert!(
            Allocation::assign(
                &[slot("a", vec![register("rax", 0), register("rax", 0)])],
                &[]
            )
            .is_err()
        );
        assert!(
            Allocation::assign(
                &[slot(
                    "a",
                    vec![register("rax", 0), register("rax", 0), register("rcx", 1)]
                )],
                &[name("rax")]
            )
            .unwrap_err()
            .contains("repeats")
        );
        assert!(
            Allocation::assign(
                &[
                    slot("a", vec![register("rax", 0)]),
                    slot("a", vec![register("rcx", 1)])
                ],
                &[]
            )
            .is_err()
        );
        assert!(
            Allocation::assign(
                &[
                    slot("a", vec![register("rax", 0)]),
                    slot("b", vec![register("rax", 1)])
                ],
                &[]
            )
            .is_err()
        );
    }
}
