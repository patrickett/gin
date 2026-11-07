//! Raw addresses carry types, not owned origins or evidence of memory validity.
use super::*;

impl<'a> Checker<'a> {
    pub(super) fn pointer_type(&mut self, pointee: SourceType) -> Result<SourceType> {
        self.intern_instance(CheckedSourceInstanceHead::Pointer, vec![pointee])
    }

    pub(super) fn pointer_syntax(
        &mut self,
        expression: &'a Expression,
        prefix: &'a PrefixedType,
    ) -> Result<()> {
        self.child(expression.span_id, prefix.keyword.span_id)?;
        self.child(expression.span_id, prefix.target.span_id)?;
        self.token(prefix.keyword.span_id, "@")?;
        if prefix.group.is_some() {
            return Err(SourceCheckError::Malformed);
        }
        let range = self.range_of(expression.span_id);
        let keyword = self.range_of(prefix.keyword.span_id);
        let target = self.range_of(prefix.target.span_id);
        if range.start != keyword.start || range.end != target.end {
            return Err(SourceCheckError::Malformed);
        }
        self.syntax_trivia(target.file, keyword.end, target.start)
    }

    pub(super) fn cast_syntax(
        &mut self,
        expression: &'a Expression,
        value: &'a Expression,
        as_keyword: SpanId,
        target: &'a Expression,
    ) -> Result<()> {
        for span in [value.span_id, as_keyword, target.span_id] {
            self.child(expression.span_id, span)?;
        }
        self.keyword(as_keyword, "as")?;
        let range = self.range_of(expression.span_id);
        let value = self.range_of(value.span_id);
        let keyword = self.range_of(as_keyword);
        let target = self.range_of(target.span_id);
        if range.start != value.start || range.end != target.end {
            return Err(SourceCheckError::Malformed);
        }
        self.syntax_trivia(target.file, value.end, keyword.start)?;
        self.syntax_trivia(target.file, keyword.end, target.start)
    }

    pub(super) fn cast_target(&mut self, expression: &'a Expression) -> Result<SourceType> {
        self.charge(self.proof.generics.len())?;
        let scope: Vec<_> = self
            .proof
            .generics
            .iter()
            .enumerate()
            .map(|(index, name)| {
                (
                    *name,
                    SourceType::Generic {
                        index: index as u32,
                    },
                )
            })
            .collect();
        self.resolve_type(expression, &scope)
    }

    pub(super) fn reconstruct(
        &mut self,
        span: SpanId,
        value: Value,
        target: SourceType,
    ) -> Result<Value> {
        self.charge(1)?;
        // Every conversion that involves an address stops being proved: the number or pointer
        // is trusted to be what the target says. Only a number viewed in a domain is checked.
        match (value.ty, target) {
            // A pointer cast to its own type changes nothing.
            (from, to) if from == to => {}
            (SourceType::Pointer { .. }, SourceType::Pointer { .. }) => {
                self.require_acceptance(span, "casting a pointer to another pointer type")?
            }
            (_, SourceType::Pointer { .. }) => {
                self.require_acceptance(span, "casting a number to a pointer")?
            }
            (SourceType::Pointer { .. }, SourceType::Integer { .. }) => {
                self.require_acceptance(span, "casting a pointer to a number")?
            }
            (SourceType::Pointer { .. }, target) if self.is_open_domain(target) => {
                self.require_acceptance(span, "casting a pointer to a number")?
            }
            (SourceType::Pointer { .. }, _) => {
                self.require_acceptance(span, "rebuilding a value from its address")?
            }
            _ => {}
        }
        let SourceType::Pointer { id } = value.ty else {
            // A number (or an open domain over one) cast to a pointer is that address: the
            // trust boundary of raw pointers, unchecked from here on.
            if matches!(target, SourceType::Pointer { .. }) {
                let number = self.open_number(value)?;
                self.scalar(number)?;
                return self.fresh_value(target);
            }
            // Anything else is a number (or a domain over one) viewed as another domain: the
            // conversion holds only where the value's range is proved to fit.
            return self.number_cast(value, target);
        };
        // An address cast to another pointer type is the same address, typed anew.
        if matches!(target, SourceType::Pointer { .. }) {
            return self.fresh_value(target);
        }
        // An address cast to a number: some number a register holds.
        if matches!(target, SourceType::Integer { .. }) || self.is_open_domain(target) {
            let bits = self.target.register_bits.min(250);
            let top = (I256::ONE << I256::from(bits)) - I256::ONE;
            let (lower, upper) = match self.domain_bounds_of(target) {
                Some((lower, upper)) => (lower, upper),
                None => match target {
                    SourceType::Integer { lower, upper } => (lower, upper),
                    _ => (I256::ZERO, top),
                },
            };
            let number = self.fresh_value(SourceType::Integer {
                lower: lower.max(I256::ZERO),
                upper: upper.min(top),
            })?;
            return self.view_as(number, target);
        }
        let instance = &self.module.instances[id as usize];
        assert_eq!(instance.head, CheckedSourceInstanceHead::Pointer);
        assert_eq!(instance.arguments.len(), 1);
        // Interning includes nominal origin, complete arguments and structural shape.
        // In particular sum inclusion and scalar widening are not raw conversions.
        if instance.arguments[0] != target {
            return Err(SourceCheckError::Type);
        }
        self.reconstruction_access(target)?;
        // Never infer a pointee owner or identity from the address identity.
        self.fresh_value(target)
    }

    /// May `#load` make a value of type `ty`? A type parameter: yes, parametrically; the value
    /// read is whatever was stored there, and the address itself was accepted with
    /// `#unchecked(...)`. A concrete type: only one whose private representation this module
    /// may build, as for rebuilding a value from its address.
    pub(super) fn load_access(&mut self, ty: SourceType) -> Result<()> {
        if matches!(ty, SourceType::Generic { .. }) {
            return Ok(());
        }
        self.reconstruction_access(ty)
    }

    fn reconstruction_access(&mut self, root: SourceType) -> Result<()> {
        self.charge(1)?;
        let mut pending = vec![root];
        let mut seen = Vec::new();
        while let Some(ty) = pending.pop() {
            self.charge(seen.len() + 1)?;
            if seen.contains(&ty) {
                continue;
            }
            seen.push(ty);
            match ty {
                SourceType::Unit
                | SourceType::Text
                | SourceType::Integer { .. }
                | SourceType::Pointer { .. } => {}
                // No authority constraint exists for an unknown contained representation.
                SourceType::Generic { .. } => return Err(SourceCheckError::Privacy),
                SourceType::Aggregate { id } => {
                    let schema = self.module.instances[id as usize].schema();
                    self.schema_access(schema)?;
                    pending.extend(self.instance_fields(id)?);
                }
                SourceType::Sum { id } => pending.extend(self.sum_variants(id)?),
                SourceType::Array { id } | SourceType::Sequence { id } => {
                    let arguments = &self.module.instances[id as usize].arguments;
                    prepay(&mut self.remaining, arguments.len())?;
                    pending.extend_from_slice(arguments);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(
        name: &str,
        text: &str,
        dependencies: &[&CheckedSourceModule],
    ) -> CheckedSourceModule {
        let source = parser::parse_strict(text).unwrap();
        check_module(name, &source, dependencies, crate::Limits::default()).unwrap()
    }

    fn returned(source: &mut SourceFile) -> &mut Expression {
        let Item::Binding(binding) = &mut source.items[1].value else {
            unreachable!()
        };
        let Expr::Block(block) = &mut binding.value.as_mut().unwrap().expression.value else {
            unreachable!()
        };
        let Statement::Return(returned) = &mut block.statements[0].value else {
            unreachable!()
        };
        returned.value.as_mut().unwrap()
    }

    #[test]
    fn pointer_values_copy_addresses_without_owners_or_pointee_children() {
        let source = parser::parse_strict("").unwrap();
        let mut checker = Checker::new(&source, crate::Limits::default());
        let pointee = SourceType::Generic { index: 0 };
        let pointer = checker.pointer_type(pointee).unwrap();
        let value = checker.input_value(pointer).unwrap();
        assert!(value.owner.is_none());
        assert!(checker.proof.owners.is_empty());
        checker.insert("address", value).unwrap();
        let first = checker.local("address", "address").unwrap();
        let second = checker.local("address", "address").unwrap();
        assert_eq!(first.identity, second.identity);
        let inputs = checker.summary_inputs().unwrap();
        let summary = checker.summarize(&inputs, Some(first)).unwrap();
        assert_eq!(summary.outputs[0].input, Some(0));
        assert_eq!(summary.dispositions, [summaries::Disposition::Scalar]);
        let actual = checker.input_value(pointer).unwrap();
        let result = checker
            .apply_summary(&summary, &[actual], &[pointee])
            .unwrap();
        assert_eq!(result.identity, actual.identity);
        assert!(result.owner.is_none());
        // Opaque pointees do not stop copying or reconstructing another pointer.
        let indirect = checker.pointer_type(pointer).unwrap();
        let address = checker.input_value(indirect).unwrap();
        checker.acceptances.push((SpanId::INVALID, false));
        let result = checker.reconstruct(SpanId::INVALID, address, pointer).unwrap();
        assert!(result.owner.is_none());
        assert_ne!(result.identity, address.identity);
    }

    #[test]
    fn reconstruction_summaries_are_fresh_and_preserve_lazy_layouts_across_import() {
        let library = module(
            "library",
            concat!(
                "Marker has\n",
                "Record has address @Marker\n",
                "rebuild(address @Record) Record: return #unchecked(address as Record)\n",
            ),
            &[],
        );
        let summary = &library.summaries("rebuild")[0];
        assert_eq!(summary.outputs.len(), 1);
        assert_eq!(summary.outputs[0].input, None);
        assert!(!summary.outputs[0].materialized);
        let client = module(
            "client",
            concat!(
                "use library Marker, Record, rebuild\n",
                "unwrap(address @Record) @Marker:\n",
                "  Record(child) := rebuild(address)\n",
                "  return child\n",
            ),
            &[&library],
        );
        let output = &client.summaries("unwrap")[0].outputs[0];
        assert_eq!(output.input, None);
        assert!(matches!(output.ty, SourceType::Pointer { .. }));
    }

    #[test]
    fn pointer_instances_and_nominal_origins_survive_compaction_and_diamond_import() {
        let origin = module(
            "origin",
            concat!(
                "Marker has\n",
                "Record has\n  #private\n  address @Marker\n",
                "adopt(address @Record) Record: return #unchecked(address as Record)\n",
            ),
            &[],
        );
        let left = module(
            "left",
            concat!(
                "use origin Record, adopt\n",
                "forward(address @Record) Record: return adopt(address)\n",
            ),
            &[&origin],
        );
        let right = module(
            "right",
            "use origin Record\nidentity(address @Record) @Record: return address\n",
            &[&origin],
        );
        let client = module(
            "client",
            concat!(
                "use left forward\n",
                "use right identity\n",
                "use origin Record\n",
                "consume(address @Record) Record: return forward(identity(address))\n",
            ),
            &[&left, &right, &origin],
        );
        assert_eq!(client.checked().aggregates.len(), 2);
        let signature = &client.checked().functions[0];
        let SourceType::Pointer { id } = signature.parameters[0] else {
            unreachable!()
        };
        let pointer = &client.checked().instances[id as usize];
        assert_eq!(pointer.head, CheckedSourceInstanceHead::Pointer);
        assert_eq!(pointer.arguments, [signature.result]);
        let summary = &client.summaries("consume")[0];
        assert_eq!(summary.outputs[0].input, None);
        // Reusing the origin's display name does not grant its construction authority.
        let source = parser::parse_strict(concat!(
            "use left forward\n",
            "Local has\n",
            "bad(address @Local) Local: return #unchecked(address as Local)\n",
        ))
        .unwrap();
        assert!(check_module("origin", &source, &[&left], crate::Limits::default()).is_ok());
        let source = parser::parse_strict(concat!(
            "use origin Record\n",
            "bad(address @Record) Record: return #unchecked(address as Record)\n",
        ))
        .unwrap();
        assert_eq!(
            check_module("client", &source, &[&origin], crate::Limits::default()).unwrap_err(),
            SourceCheckError::Privacy
        );
    }

    #[test]
    fn reconstruction_traversal_budgets_fail_closed_including_after_substitution() {
        let source = parser::parse_strict(concat!(
            "Marker has\n",
            "Box(element) has child element\n",
            "rebuild(address @Box((Marker, [Marker; 2]))) Box((Marker, [Marker; 2])):\n",
            "  return #unchecked(address as Box((Marker, [Marker; 2])))\n",
        ))
        .unwrap();
        let mut lower = 0;
        let mut upper = crate::Limits::default().work_max;
        while lower < upper {
            let middle = lower + (upper - lower) / 2;
            let limits = crate::Limits {
                work_max: middle,
                ..crate::Limits::default()
            };
            match check_file(&source, limits) {
                Ok(_) => upper = middle,
                Err(SourceCheckError::Budget) => lower = middle + 1,
                other => panic!("{other:?}"),
            }
        }
        assert!(lower > 0);
        assert_eq!(
            check_file(
                &source,
                crate::Limits {
                    work_max: lower - 1,
                    ..crate::Limits::default()
                }
            ),
            Err(SourceCheckError::Budget)
        );
        assert!(
            check_file(
                &source,
                crate::Limits {
                    work_max: lower,
                    ..crate::Limits::default()
                }
            )
            .is_ok()
        );
        assert_eq!(
            check_file(
                &source,
                crate::Limits {
                    flow_work_max: 0,
                    ..crate::Limits::default()
                }
            ),
            Err(SourceCheckError::Budget)
        );
    }

    #[test]
    fn fresh_sum_summaries_keep_alternatives_lazy_and_source_free() {
        let library = module(
            "library",
            concat!(
                "Marker has\n",
                "First has address @Marker\n",
                "Second has address @Marker\n",
                "Choice is First(First) or Second(Second)\n",
                "rebuild(address @Choice) Choice: return #unchecked(address as Choice)\n",
            ),
            &[],
        );
        let output = &library.summaries("rebuild")[0].outputs[0];
        assert!(matches!(output.ty, SourceType::Sum { .. }));
        assert_eq!(output.input, None);
        assert!(!output.materialized);
        let client = module(
            "client",
            concat!(
                "use library Marker, First, Second, Choice, rebuild\n",
                "read(address @Choice) @Marker:\n",
                "  return when rebuild(address) is\n",
                "    First(first) then\n",
                "      First(child) := first\n",
                "      return child\n",
                "    Second(second) then\n",
                "      Second(child) := second\n",
                "      return child\n",
            ),
            &[&library],
        );
        assert_eq!(client.summaries("read").len(), 1);
        assert_eq!(client.summaries("read")[0].outputs[0].input, None);
    }

    #[test]
    fn pointer_fields_bound_recursive_layouts_and_authority_traversal() {
        let library = module(
            "library",
            concat!(
                "Node has next @Node\n",
                "rebuild(address @Node) Node: return #unchecked(address as Node)\n",
            ),
            &[],
        );
        let client = module(
            "client",
            concat!(
                "use library Node, rebuild\n",
                "next(address @Node) @Node:\n",
                "  Node(child) := rebuild(address)\n",
                "  return child\n",
            ),
            &[&library],
        );
        assert!(matches!(
            client.checked().functions[0].result,
            SourceType::Pointer { .. }
        ));
    }

    #[test]
    fn manual_cast_nodes_validate_tokens_order_exact_edges_and_trivia() {
        for mutation in 0..7 {
            let mut source = parser::parse_strict(
                "Marker has\nf(p @Marker) Marker: return p as Marker\n",
            )
            .unwrap();
            let span = returned(&mut source).span_id;
            let range = source.span_table.get(span).to_range();
            let expression = returned(&mut source);
            let Expr::Cast {
                value,
                as_keyword,
                target,
            } = &mut expression.value
            else {
                unreachable!()
            };
            match mutation {
                0 => *as_keyword = SpanId::INVALID,
                1 => std::mem::swap(&mut value.span_id, &mut target.span_id),
                2 => expression.span_id = target.span_id,
                3 => value.span_id = expression.span_id,
                4 => *as_keyword = target.span_id,
                5 => {
                    let span = source.span_table.insert_range(range.start..range.end + 1);
                    returned(&mut source).span_id = span;
                }
                6 => {
                    let position = source.text.find(" as ").unwrap();
                    let mut text = source.text.to_string();
                    text.replace_range(position..position + 1, "+");
                    source.text = text.into();
                }
                _ => unreachable!(),
            }
            assert_eq!(
                check_file(&source, crate::Limits::default()),
                Err(SourceCheckError::Malformed),
                "mutation {mutation}"
            );
        }
    }

    #[test]
    fn manual_pointer_nodes_cannot_erase_prefix_or_inject_nontrivia() {
        for mutation in 0..5 {
            let mut source = parser::parse_strict(
                "Marker has\nf(p @ Marker) Marker: return p as Marker\n",
            )
            .unwrap();
            let Item::Binding(binding) = &mut source.items[1].value else {
                unreachable!()
            };
            let annotation = binding.parameters.as_mut().unwrap().value[0]
                .annotation
                .as_mut()
                .unwrap();
            let Annotation::Expression(expression) = &mut annotation.value else {
                unreachable!()
            };
            let Expr::PrefixedType(prefix) = &mut expression.value else {
                unreachable!()
            };
            match mutation {
                0 => prefix.keyword.span_id = SpanId::INVALID,
                1 => prefix.keyword.span_id = prefix.target.span_id,
                2 => expression.span_id = prefix.target.span_id,
                3 => expression.value = prefix.target.value.clone(),
                4 => {
                    let position = source.text.find("@ ").unwrap() + 1;
                    let mut text = source.text.to_string();
                    text.replace_range(position..position + 1, "+");
                    source.text = text.into();
                }
                _ => unreachable!(),
            }
            assert_eq!(
                check_file(&source, crate::Limits::default()),
                Err(SourceCheckError::Malformed),
                "mutation {mutation}"
            );
        }
    }
}
