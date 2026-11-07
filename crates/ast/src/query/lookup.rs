mod annotations;
mod collections;
mod domains;
mod imports;
mod type_prefix;

use crate::{
    Argument, Binding, ClauseItem, Expr, Expression, Item, Name, Parameter, SourceFile, Span, SpanId, Spanned,
    Statement, TypeDeclaration, TypeMember,
};

use super::{Declaration, NodeAtOffset, SyntaxNode, SyntaxNodeKind as Kind};

#[derive(Clone, Copy)]
struct Record<'syntax> {
    node: SyntaxNode<'syntax>,
    parent: Option<usize>,
    depth: usize,
}

struct Lookup<'syntax> {
    file: &'syntax SourceFile,
    offset_bytes: usize,
    end_touch: bool,
    records: Vec<Record<'syntax>>,
    best: usize,
}

impl SourceFile {
    /// Find the deepest source node at a UTF-8 byte insertion point.
    ///
    /// Character positions use half-open ranges. Immediately before ASCII
    /// whitespace, or at EOF, syntax ending at the cursor can retain context.
    /// A zero-width recovery node wins at its exact insertion point. Otherwise
    /// equal-range wrappers select their deepest leaf, with source order breaking
    /// equal-depth ties. Empty files and whitespace outside syntax select `File`.
    ///
    /// Returns `None` for an out-of-bounds offset or one splitting a UTF-8 scalar.
    /// This is not an LSP UTF-16 position converter. Comments remain separate file
    /// metadata; querying inside one returns the enclosing syntax, not a fake name.
    ///
    /// The query assumes the source AST's documented span/containment invariants.
    /// Traversal is iterative, prunes non-touching branches, and never clones AST
    /// bodies. It scans sibling lists; no edit-stable identity or index is implied.
    pub fn node_at_offset(&self, offset_bytes: usize) -> Option<NodeAtOffset<'_>> {
        self.text.get(..offset_bytes)?;
        Some(Lookup::new(self, offset_bytes).run())
    }
}

impl<'syntax> Lookup<'syntax> {
    fn new(file: &'syntax SourceFile, offset_bytes: usize) -> Self {
        assert!(offset_bytes <= file.text.len());
        assert!(file.text.is_char_boundary(offset_bytes));
        assert!(u32::try_from(file.text.len()).is_ok());
        let node = SyntaxNode {
            kind: Kind::File,
            span: Span::new(0, file.text.len()),
        };
        let end_touch = file
            .text
            .as_bytes()
            .get(offset_bytes)
            .is_none_or(u8::is_ascii_whitespace);
        Self {
            file,
            offset_bytes,
            end_touch,
            records: vec![Record {
                node,
                parent: None,
                depth: 0,
            }],
            best: 0,
        }
    }

    fn run(mut self) -> NodeAtOffset<'syntax> {
        let mut index = 0;
        // Each record corresponds to a distinct view of the finite, owned AST.
        // Children can only refer to an already-recorded parent, never a cycle.
        while index < self.records.len() {
            self.select(index);
            self.children(index);
            index += 1;
        }
        self.finish()
    }

    fn add(&mut self, parent: usize, kind: Kind<'syntax>, id: SpanId) {
        assert!(parent < self.records.len());
        assert!(id.is_valid());
        let span = self.file.span_table.get(id);
        let enclosing = self.records[parent].node.span;
        assert!(self.file.text.get(span.to_range()).is_some());
        assert!(enclosing.start() <= span.start());
        assert!(span.end() <= enclosing.end());
        // End-touching ancestors must be visited even when they cannot themselves
        // win: a zero-width missing child may sit exactly at their exclusive end.
        if span.start() <= self.offset_bytes && self.offset_bytes <= span.end() {
            let depth = self.records[parent].depth + 1;
            self.records.push(Record {
                node: SyntaxNode { kind, span },
                parent: Some(parent),
                depth,
            });
        }
    }

    fn select(&mut self, index: usize) {
        let record = self.records[index];
        let span = record.node.span;
        let eligible = span.is_empty()
            || span.contains(self.offset_bytes)
            || self.end_touch && span.end() == self.offset_bytes;
        if eligible {
            let best = self.records[self.best];
            if (span.is_empty(), record.depth) > (best.node.span.is_empty(), best.depth) {
                self.best = index;
            }
        }
    }

    fn finish(self) -> NodeAtOffset<'syntax> {
        let mut path = Vec::new();
        let mut index = self.best;
        for _ in 0..self.records.len() {
            let record = self.records[index];
            path.push(record.node);
            match record.parent {
                Some(parent) => {
                    assert!(parent < index);
                    index = parent;
                }
                None => break,
            }
        }
        assert!(matches!(
            path.last().map(|node| node.kind),
            Some(Kind::File)
        ));
        path.reverse();
        let declarations = path
            .iter()
            .filter_map(|node| match node.kind {
                Kind::Binding(binding) => {
                    Some(Declaration::from_binding(self.file, node.span, binding))
                }
                Kind::TypeDeclaration(declaration) => {
                    Some(Declaration::from_type(self.file, node.span, declaration))
                }
                Kind::PatternBinder(name) => Some(super::outline::Declaration {
                    name: &name.value,
                    kind: super::outline::DeclarationKind::PatternBinder,
                    span: node.span,
                    name_span: node.span,
                }),
                _ => None,
            })
            .collect();
        let node = path.pop().expect("the file root is always a candidate");
        NodeAtOffset {
            node,
            ancestors: path,
            declarations,
        }
    }

    fn children(&mut self, index: usize) {
        match self.records[index].node.kind {
            Kind::File => self.file_children(index),
            Kind::Item(item) => self.item_children(index, item),
            Kind::Import(import) => {
                for entry in &import.entries {
                    self.add(index, Kind::ImportEntry(entry), entry.span_id);
                }
            }
            Kind::ImportEntry(entry) => self.import_entry_children(index, &entry.value),
            Kind::ImportSource(source) => self.import_source_children(index, &source.value),
            Kind::ImportMember(member) => self.import_member_children(index, &member.value),
            Kind::Binding(binding) => self.binding_children(index, binding),
            Kind::FieldAttribute(attribute) => match &attribute.value {
                crate::FieldAttribute::Private {
                    hash,
                    private_keyword,
                } => {
                    self.add(index, Kind::AttributeHash, *hash);
                    self.add(index, Kind::PrivateKeyword, *private_keyword);
                }
                crate::FieldAttribute::Infix {
                    hash,
                    keyword,
                    operator,
                    ..
                } => {
                    self.add(index, Kind::AttributeHash, *hash);
                    self.add(index, Kind::InfixKeyword, *keyword);
                    self.add(index, Kind::InfixOperator, *operator);
                }
                crate::FieldAttribute::Inline { hash, keyword } => {
                    self.add(index, Kind::AttributeHash, *hash);
                    self.add(index, Kind::InlineKeyword, *keyword);
                }
                crate::FieldAttribute::Register { hash, .. } => {
                    self.add(index, Kind::AttributeHash, *hash);
                }
            },
            Kind::Destructure(pattern) => {
                self.add(
                    index,
                    Kind::VariantPattern(&pattern.pattern.value),
                    pattern.pattern.span_id,
                );
                self.add(
                    index,
                    Kind::BindingOperator(&pattern.operator),
                    pattern.operator.span_id,
                );
                self.expression(index, &pattern.value);
            }
            Kind::MatchPattern(pattern) => match &pattern.value {
                crate::MatchPattern::Variant(variant) => {
                    self.add(index, Kind::VariantPattern(variant), pattern.span_id);
                }
                crate::MatchPattern::Integer(expression) => self.expression(index, expression),
                crate::MatchPattern::Domain { domain, .. } => self.expression(index, domain),
                crate::MatchPattern::Text(expression) => self.expression(index, expression),
                crate::MatchPattern::Else { .. } => {}
            },
            Kind::VariantPattern(pattern) => {
                self.name(index, &pattern.constructor);
                if let Some(fields) = &pattern.fields {
                    self.add(
                        index,
                        Kind::PatternBinderList(fields),
                        fields.binders.span_id,
                    );
                }
            }
            Kind::Match(case) => {
                self.add(index, Kind::WhenKeyword, case.when_keyword);
                self.expression(index, &case.subject);
                self.add(index, Kind::IsKeyword, case.is_keyword);
                self.add(index, Kind::MatchArmList(&case.arms), case.arms.span_id);
            }
            Kind::Assign(statement) => {
                self.expression(index, &statement.place);
                if let Some(compound) = &statement.compound {
                    self.add(index, Kind::ReassignOperator, compound.span_id);
                }
                self.add(index, Kind::ReassignOperator, statement.operator);
                self.expression(index, &statement.value);
            }
            Kind::Conditional(case) => {
                self.add(index, Kind::WhenKeyword, case.when_keyword);
                self.expression(index, &case.condition);
                self.add(index, Kind::ThenKeyword, case.then_keyword);
                self.expression(index, &case.then_branch);
                self.add(index, Kind::ElseKeyword, case.else_keyword);
                self.expression(index, &case.else_branch);
            }
            Kind::Chain(chain) => {
                self.add(index, Kind::WhenKeyword, chain.when_keyword);
                for arm in &chain.arms {
                    self.expression(index, &arm.value.condition);
                    self.add(index, Kind::ThenKeyword, arm.value.then_keyword);
                    self.expression(index, &arm.value.value);
                }
                self.add(index, Kind::ElseKeyword, chain.else_keyword);
                self.expression(index, &chain.otherwise);
            }
            Kind::MatchArmList(arms) => {
                for arm in &arms.value {
                    self.add(index, Kind::MatchArm(arm), arm.span_id);
                }
            }
            Kind::MatchArm(arm) => {
                self.add(
                    index,
                    Kind::MatchPattern(&arm.value.pattern),
                    arm.value.pattern.span_id,
                );
                if let Some(guard) = &arm.value.guard {
                    self.add(index, Kind::AndKeyword, guard.and_keyword);
                    self.expression(index, &guard.condition);
                }
                self.add(index, Kind::ThenKeyword, arm.value.then_keyword);
                self.expression(index, &arm.value.body);
            }
            Kind::PatternBinderList(pattern) => {
                self.add(
                    index,
                    Kind::PatternDelimiter(crate::GroupDelimiter::Open),
                    pattern.open,
                );
                for (at, (binder, selector)) in pattern.binders.value.iter().zip(&pattern.selectors).enumerate() {
                    // A pattern written in place of a binder is a pattern of its own.
                    if let Some(Some(inner)) = pattern.nested.get(at) {
                        self.add(index, Kind::VariantPattern(&inner.value), inner.span_id);
                        if let Some(selector) = selector {
                            self.add(index, Kind::PatternSelector(selector), selector.span_id);
                        }
                        continue;
                    }
                    self.add(index, Kind::PatternBinder(binder), binder.span_id);
                    if let Some(selector) = selector {
                        self.add(index, Kind::PatternSelector(selector), selector.span_id);
                    }
                }
                self.add(
                    index,
                    Kind::PatternDelimiter(crate::GroupDelimiter::Close),
                    pattern.close,
                );
            }
            Kind::TypeDeclaration(declaration) => self.type_children(index, declaration),
            Kind::ParameterList(parameters) => {
                for parameter in &parameters.value {
                    self.add(index, Kind::Parameter(parameter), parameter.span_id);
                }
            }
            Kind::Parameter(parameter) => self.parameter_children(index, parameter),
            Kind::ParameterConvention(convention) => {
                self.convention_children(index, &convention.value);
            }
            Kind::GroupPath(group) => self.group_children(index, &group.value),
            Kind::Annotation(annotation) => self.annotation_children(index, &annotation.value),
            Kind::Expression(expression) => self.expression_children(index, expression),
            Kind::Array(array) => self.array_children(index, array),
            Kind::Sequence(sequence) => self.sequence_children(index, sequence),
            Kind::Repeat(repeat) => self.repeat_children(index, repeat),
            Kind::Select(select) => self.select_children(index, select),
            Kind::NumericSelect(select) => self.numeric_select_children(index, select),
            Kind::Loop(loop_expression) => self.loop_children(index, loop_expression),
            Kind::ArgumentList(arguments) => {
                for argument in &arguments.value {
                    self.add(index, Kind::Argument(argument), argument.span_id);
                }
            }
            Kind::Argument(argument) => self.argument_children(index, argument),
            Kind::Statement(statement) => self.statement_children(index, statement),
            Kind::ExtraResult(extra) => {
                self.name(index, &extra.value.name);
                let annotation = &extra.value.annotation;
                self.add(index, Kind::Annotation(annotation), annotation.span_id);
            }
            Kind::ResultAlternative(alternative) => {
                self.add(index, Kind::OrKeyword, alternative.value.or_keyword);
                let annotation = &alternative.value.annotation;
                self.add(index, Kind::Annotation(annotation), annotation.span_id);
                if let Some(contract) = &alternative.value.contract {
                    self.add(index, Kind::Contract(contract), contract.span_id);
                }
            }
            Kind::Contract(contract) => {
                self.add(index, Kind::WhereKeyword, contract.value.where_keyword);
                for item in &contract.value.items {
                    match &item.value {
                        ClauseItem::Transition(_) => {
                            self.add(index, Kind::Transition(item), item.span_id);
                        }
                        ClauseItem::Claim(claim) => self.predicate_children(index, claim),
                    }
                }
            }
            Kind::Transition(item) => {
                if let ClauseItem::Transition(transition) = &item.value {
                    self.expression(index, &transition.place);
                    self.add(index, Kind::ReassignOperator, transition.operator);
                    self.expression(index, &transition.value);
                }
            }
            Kind::TypeBody(body) => self.type_body_children(index, &body.value),
            Kind::TypeDomain(domain) => self.domain_children(index, &domain.value),
            Kind::DomainBase(base) => self.domain_base_children(index, &base.value),
            Kind::Refinement(refinement) => self.refinement_children(index, &refinement.value),
            Kind::RefinementPredicate(predicate) => {
                self.predicate_children(index, &predicate.value);
            }
            Kind::TypeMember(member) => match &member.value {
                TypeMember::Binding(binding) => {
                    self.add(index, Kind::Binding(binding), member.span_id);
                }
                TypeMember::Provision(_) | TypeMember::Error => {}
                TypeMember::ProvisionApplied(expression) => self.expression(index, expression),
            },
            Kind::Name(_)
            | Kind::LoopBinder(_)
            | Kind::LoopKeyword(_)
            | Kind::RangeKeyword
            | Kind::AsKeyword
            | Kind::CollectionPunctuation(_)
            | Kind::PatternBinder(_)
            | Kind::PatternSelector(_)
            | Kind::PatternDelimiter(_)
            | Kind::WhenKeyword
            | Kind::ElseKeyword
            | Kind::ReassignOperator
            | Kind::WhereKeyword
            | Kind::IsKeyword
            | Kind::ThenKeyword
            | Kind::AndKeyword
            | Kind::AttributeHash
            | Kind::PrivateKeyword
            | Kind::InfixKeyword
            | Kind::InlineKeyword
            | Kind::InfixOperator
            | Kind::ResultBinder(_)
            | Kind::OrKeyword
            | Kind::BindingConvention(_)
            | Kind::Convention(_)
            | Kind::TypePrefixKeyword(_)
            | Kind::GroupDelimiter(_)
            | Kind::BinaryOperator(_)
            | Kind::BindingOperator(_)
            | Kind::UnaryOperator(_)
            | Kind::AnnotationKeyword(_)
            | Kind::DomainKeyword(_) => {}
        }
    }

    fn file_children(&mut self, index: usize) {
        let file = self.file;
        for item in &file.items {
            self.add(index, Kind::Item(item), item.span_id);
        }
    }

    fn item_children(&mut self, index: usize, item: &'syntax Spanned<Item>) {
        match &item.value {
            Item::Import(import) => self.add(index, Kind::Import(import), item.span_id),
            Item::Binding(binding) => self.add(index, Kind::Binding(binding), item.span_id),
            Item::Type(declaration) => {
                self.add(index, Kind::TypeDeclaration(declaration), item.span_id);
            }
            Item::Expression(expression) => self.expression(index, expression),
            Item::Private
            | Item::Entry
            | Item::Load
            | Item::Store
            | Item::Wrap
            | Item::Unrestricted
            | Item::Instruction { .. }
            | Item::Error => {}
        }
    }

    fn binding_children(&mut self, index: usize, binding: &'syntax Binding) {
        for attribute in &binding.attributes {
            self.add(index, Kind::FieldAttribute(attribute), attribute.span_id);
        }
        self.name(index, &binding.name);
        if let Some(parameters) = &binding.parameters {
            self.add(index, Kind::ParameterList(parameters), parameters.span_id);
        }
        if let Some(convention) = &binding.convention {
            self.add(
                index,
                Kind::BindingConvention(convention),
                convention.span_id,
            );
        }
        if let Some(name) = &binding.result_binder {
            self.add(index, Kind::ResultBinder(name), name.span_id);
        }
        if let Some(annotation) = &binding.annotation {
            self.add(index, Kind::Annotation(annotation), annotation.span_id);
        }
        if let Some(contract) = &binding.contract {
            self.add(index, Kind::Contract(contract), contract.span_id);
        }
        for extra in &binding.extra_results {
            self.add(index, Kind::ExtraResult(extra), extra.span_id);
        }
        for alternative in &binding.alternatives {
            self.add(index, Kind::ResultAlternative(alternative), alternative.span_id);
        }
        if let Some(value) = &binding.value {
            self.add(
                index,
                Kind::BindingOperator(&value.operator),
                value.operator.span_id,
            );
            self.expression(index, &value.expression);
        }
    }

    fn type_children(&mut self, index: usize, declaration: &'syntax TypeDeclaration) {
        self.name(index, &declaration.name);
        if let Some(parameters) = &declaration.parameters {
            self.add(index, Kind::ParameterList(parameters), parameters.span_id);
        }
        self.add(
            index,
            Kind::TypeBody(&declaration.body),
            declaration.body.span_id,
        );
    }

    fn parameter_children(&mut self, index: usize, parameter: &'syntax Spanned<Parameter>) {
        self.name(index, &parameter.value.name);
        if let Some(convention) = &parameter.value.convention {
            self.add(
                index,
                Kind::ParameterConvention(convention),
                convention.span_id,
            );
        }
        if let Some(annotation) = &parameter.value.annotation {
            self.add(index, Kind::Annotation(annotation), annotation.span_id);
        }
        if let Some(default) = &parameter.value.default {
            self.expression(index, default);
        }
    }

    fn expression(&mut self, parent: usize, expression: &'syntax Expression) {
        self.add(parent, Kind::Expression(expression), expression.span_id);
    }

    fn name(&mut self, parent: usize, name: &'syntax Name) {
        self.add(parent, Kind::Name(&name.value), name.span_id);
    }

    fn expression_children(&mut self, index: usize, expression: &'syntax Expression) {
        match &expression.value {
            Expr::Name(name) => self.add(index, Kind::Name(name), expression.span_id),
            Expr::Array(array) => self.add(index, Kind::Array(array), expression.span_id),
            Expr::Sequence(sequence) => {
                self.add(index, Kind::Sequence(sequence), expression.span_id)
            }
            Expr::Repeat(repeat) => self.add(index, Kind::Repeat(repeat), expression.span_id),
            Expr::Select(select) => self.add(index, Kind::Select(select), expression.span_id),
            Expr::NumericSelect(select) => {
                self.add(index, Kind::NumericSelect(select), expression.span_id);
            }
            Expr::Loop(loop_expression) => {
                self.add(index, Kind::Loop(loop_expression), expression.span_id);
            }
            Expr::Range {
                start,
                to_keyword,
                end,
            } => {
                self.expression(index, start);
                self.add(index, Kind::RangeKeyword, *to_keyword);
                self.expression(index, end);
            }
            Expr::PrefixedType(prefix) => self.prefix_children(index, prefix),
            Expr::Cast {
                value,
                as_keyword,
                target,
            } => {
                self.expression(index, value);
                self.add(index, Kind::AsKeyword, *as_keyword);
                self.expression(index, target);
            }
            Expr::Member { base, name } => {
                self.expression(index, base);
                self.name(index, name);
            }
            Expr::IncompleteMember { base } => self.expression(index, base),
            Expr::Unary { operator, operand } => {
                self.add(index, Kind::UnaryOperator(operator), operator.span_id);
                self.expression(index, operand);
            }
            Expr::Call { callee, arguments } => {
                self.expression(index, callee);
                self.add(index, Kind::ArgumentList(arguments), arguments.span_id);
            }
            Expr::Binary {
                left,
                operator,
                right,
            } => {
                self.expression(index, left);
                self.add(index, Kind::BinaryOperator(operator), operator.span_id);
                self.expression(index, right);
            }
            Expr::Group(inner) => self.expression(index, inner),
            Expr::Bind(binding) => self.add(index, Kind::Binding(binding), expression.span_id),
            Expr::Match(case) => self.add(index, Kind::Match(case), expression.span_id),
            Expr::IsTest(test) => {
                self.expression(index, &test.subject);
            }
            Expr::Conditional(case) => {
                self.add(index, Kind::Conditional(case), expression.span_id);
            }
            Expr::Chain(chain) => self.add(index, Kind::Chain(chain), expression.span_id),
            Expr::Block(block) => {
                for statement in &block.statements {
                    self.add(index, Kind::Statement(statement), statement.span_id);
                }
            }
            Expr::Query { name, .. } => self.name(index, name),
            Expr::Literal(_) | Expr::Hole | Expr::Missing => {}
        }
    }

    fn argument_children(&mut self, index: usize, argument: &'syntax Spanned<Argument>) {
        match &argument.value {
            Argument::Positional(value) => self.expression(index, value),
            Argument::Named { name, value } => {
                self.name(index, name);
                self.expression(index, value);
            }
            Argument::Typed { name, annotation } => {
                self.name(index, name);
                self.expression(index, annotation);
            }
        }
    }

    fn statement_children(&mut self, index: usize, statement: &'syntax Spanned<Statement>) {
        match &statement.value {
            Statement::Destructure(pattern) => {
                self.add(index, Kind::Destructure(pattern), statement.span_id);
            }
            Statement::Assign(assign) => {
                self.add(index, Kind::Assign(assign), statement.span_id);
            }
            Statement::Binding(binding) => {
                self.add(index, Kind::Binding(binding), statement.span_id)
            }
            Statement::Expression(expression) => self.expression(index, expression),
            Statement::Return(returned) => {
                if let Some(value) = &returned.value {
                    self.expression(index, value);
                }
            }
            Statement::When(when) => {
                self.expression(index, &when.condition);
                let else_block = when.else_branch.iter().map(|branch| &branch.block);
                for block in std::iter::once(&when.then_block).chain(else_block) {
                    for statement in &block.value.statements {
                        self.add(index, Kind::Statement(statement), statement.span_id);
                    }
                }
            }
            Statement::WhenIs(when) => {
                self.expression(index, &when.subject);
                let else_block = when.else_branch.iter().map(|branch| &branch.block);
                for block in std::iter::once(&when.then_block).chain(else_block) {
                    for statement in &block.value.statements {
                        self.add(index, Kind::Statement(statement), statement.span_id);
                    }
                }
            }
            Statement::Error => {}
        }
    }
}
