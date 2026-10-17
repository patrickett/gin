use std::collections::HashMap;

use ast::parameter::ParameterKind;
use ast::ty::UnionVariant;
use ast::{Pattern, Ty};
use i256::I256;
use internment::Intern;

use super::when_exhaustive::{parameter_covers_type, pattern_covers_union_variant};

type PayloadRow<'a> = Vec<(Intern<String>, &'a ParameterKind)>;

pub(crate) fn patterns_cover_union_variant(
    patterns: &[&Pattern],
    variant: &UnionVariant,
    registry: Option<&crate::TypeRegistry>,
) -> bool {
    if patterns
        .iter()
        .any(|pattern| pattern_covers_union_variant(pattern, variant, registry))
    {
        return true;
    }
    let rows: Vec<PayloadRow<'_>> = patterns
        .iter()
        .filter_map(|pattern| {
            let Pattern::Generic {
                name,
                params,
                param_spans,
                ..
            } = pattern
            else {
                return None;
            };
            if *name != variant.name || param_spans.len() != variant.fields.len() {
                return None;
            }
            param_spans
                .iter()
                .map(|(name, _)| {
                    params
                        .iter()
                        .find(|(parameter, _)| parameter == name)
                        .map(|(_, kind)| (*name, kind))
                })
                .collect::<Option<PayloadRow<'_>>>()
        })
        .collect();
    let columns = variant
        .fields
        .iter()
        .enumerate()
        .map(|(column, (_, ty))| {
            let ty = registry
                .map(|registry| registry.resolved_definition_for_type(ty))
                .unwrap_or_else(|| (**ty).clone());
            field_partitions(&ty, &rows, column)
        })
        .collect();
    let active = (0..rows.len()).collect::<Vec<_>>();
    PayloadCoverage {
        rows,
        columns,
        memo: HashMap::new(),
        registry,
    }
    .covers(0, &active)
}

struct PayloadCoverage<'a> {
    rows: Vec<PayloadRow<'a>>,
    columns: Vec<Vec<Ty>>,
    memo: HashMap<(usize, Vec<usize>), bool>,
    registry: Option<&'a crate::TypeRegistry>,
}

impl PayloadCoverage<'_> {
    fn covers(&mut self, column: usize, active: &[usize]) -> bool {
        if active.is_empty() {
            return false;
        }
        if column == self.columns.len() {
            return true;
        }
        let key = (column, active.to_vec());
        if let Some(covered) = self.memo.get(&key) {
            return *covered;
        }
        let groups: Vec<Vec<usize>> = self.columns[column]
            .iter()
            .map(|ty| {
                active
                    .iter()
                    .copied()
                    .filter(|row| {
                        let (name, kind) = self.rows[*row][column];
                        parameter_covers_type(&name, kind, ty, self.registry)
                    })
                    .collect()
            })
            .collect();
        let covered = groups.iter().all(|rows| self.covers(column + 1, rows));
        self.memo.insert(key, covered);
        covered
    }
}

fn field_partitions(ty: &Ty, rows: &[PayloadRow<'_>], column: usize) -> Vec<Ty> {
    if let Some(bounds) = ty.int_bounds() {
        let min = I256::from(bounds.min);
        let max = I256::from(bounds.max);
        let mut cuts = vec![min, max + I256::from(1)];
        for row in rows {
            if let Some((low, high)) = parameter_interval(row[column].1) {
                let low = low.max(min);
                let high = high.min(max);
                if low <= high {
                    cuts.push(low);
                    cuts.push(high + I256::from(1));
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        return cuts
            .windows(2)
            .map(|cut| Ty::bounded_int(cut[0], cut[1] - I256::from(1)))
            .collect();
    }
    match ty {
        Ty::Union {
            literal_values: Some(values),
            ..
        } => values.iter().cloned().map(Ty::Literal).collect(),
        Ty::Union {
            name,
            variants,
            resolved_params,
            ..
        } => variants
            .iter()
            .map(|variant| Ty::Union {
                name: *name,
                variants: vec![variant.clone()],
                literal_values: None,
                resolved_params: resolved_params.clone(),
            })
            .collect(),
        _ => vec![ty.clone()],
    }
}

fn parameter_interval(kind: &ParameterKind) -> Option<(I256, I256)> {
    let pattern = match kind {
        ParameterKind::Generic | ParameterKind::Proposition(_) => return None,
        ParameterKind::Tagged(expr)
        | ParameterKind::ValueParam { ty: expr }
        | ParameterKind::Inferred { ty: expr } => Pattern::from_expr(expr.value.clone()),
        ParameterKind::Default(expr) => Pattern::from_expr(expr.value.clone()),
    };
    match pattern {
        Pattern::InRange {
            bounds: ast::InRangeBounds::Literal(low, high),
            ..
        } => Some((low, high)),
        Pattern::Literal(ast::Literal::Int(value), _) => Some((value, value)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(value: I256) -> ParameterKind {
        ParameterKind::Default(Box::new(ast::expr::Typed::new(
            ast::expr::Expr::Lit(ast::Literal::Int(value)),
            ast::TyState::Infer,
            ast::SpanId::INVALID,
        )))
    }

    #[test]
    fn integer_partitions_clip_before_advancing_the_upper_endpoint() {
        let name = Intern::from_ref("value");
        let maximum = literal(I256::from(i128::MAX));
        let outside = literal(I256::MAX);
        let ty = Ty::bounded_int(I256::from(i128::MIN), I256::from(i128::MAX));
        let parts = field_partitions(&ty, &[vec![(name, &maximum)], vec![(name, &outside)]], 0);
        assert_eq!(parts.len(), 2);
        let first = parts[0].int_bounds().unwrap();
        let last = parts[1].int_bounds().unwrap();
        assert_eq!((first.min, first.max), (i128::MIN, i128::MAX - 1));
        assert_eq!((last.min, last.max), (i128::MAX, i128::MAX));
    }

    #[test]
    fn constructor_name_partitions_cover_a_finite_payload_union() {
        let name = Intern::from_ref("Choice");
        let payload = Ty::Union {
            name: Intern::from_ref("Bool"),
            variants: ["True", "False"]
                .map(|name| UnionVariant::new(Intern::from_ref(name), vec![]))
                .to_vec(),
            literal_values: None,
            resolved_params: None,
        };
        let variant = UnionVariant::new(name, vec![(Intern::from_ref("value"), Box::new(payload))]);
        let pattern = |parameter: &str| {
            let parameter = Intern::from_ref(parameter);
            Pattern::Generic {
                name,
                params: vec![(parameter, ParameterKind::Generic)],
                param_spans: vec![(parameter, ast::SpanId::INVALID)],
                span: ast::SpanId::INVALID,
            }
        };
        let yes = pattern("True");
        let no = pattern("False");
        assert!(!patterns_cover_union_variant(&[&yes], &variant, None));
        assert!(patterns_cover_union_variant(&[&yes, &no], &variant, None));
    }
}
