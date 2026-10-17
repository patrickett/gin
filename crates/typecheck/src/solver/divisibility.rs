use super::*;

impl ProofQuery<'_> {
    pub(super) fn prove_divisible(
        &self,
        value: &NormalExpr,
        divisor: &NormalExpr,
        substitution: &DepSubst,
    ) -> ProveResult {
        let value = substitution.apply_to_normal(value).normalize();
        let divisor = substitution.apply_to_normal(divisor).normalize();
        let NormalExpr::Value(ConstValue::Int(divisor)) = divisor else {
            return ProveResult::Unknown;
        };
        if divisor <= 0.into() {
            return ProveResult::Disproven;
        }
        if let NormalExpr::Value(ConstValue::Int(value)) = value {
            return if value % divisor == 0.into() {
                ProveResult::Proven
            } else {
                ProveResult::Disproven
            };
        }
        if divisor == 1.into() {
            return ProveResult::Proven;
        }
        for (known_value, known_divisor) in &self.environment.known_divisible {
            let known_divisor = substitution.apply_to_normal(known_divisor).normalize();
            if let NormalExpr::Value(ConstValue::Int(known_divisor)) = known_divisor
                && known_divisor > 0.into()
                && known_divisor % divisor == 0.into()
                && self.prove_eq(&value, known_value, substitution) == ProveResult::Proven
            {
                return ProveResult::Proven;
            }
        }
        ProveResult::Unknown
    }
}
