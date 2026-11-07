//! Explicit body stack: parsing nested declarations does not recurse.

use diagnostic::Diagnostic;
use diagnostic::code::SyntaxCode;
use ast::{
    Binding, BindingOperator, BindingValue, Block, ElseBranch, Expr, FieldAttribute,
    Item, ReturnStatement, SpanId, Spanned, Statement, TypeMember,
};
use lexer::TokenKind as Token;

use crate::ParseFailure;
use crate::cursor::{Cursor, ParseError, ParseResult};
use crate::declaration::{BindingContext, ParsedBinding};
use crate::when::WhenHead;
use crate::types::{ParsedType, PendingHas};

pub(crate) struct PendingBody {
    pub binding: Binding,
    pub operator: Spanned<BindingOperator>,
    pub statements: Vec<Spanned<Statement>>,
}

enum Attributed {
    Complete(Spanned<Binding>),
    Body(PendingBody),
}

/// A `when condition then` statement whose branch blocks are still being read. The statements
/// collected so far belong to the then-block, or to the else-block once `else_keyword` is set.
pub(crate) struct PendingWhen {
    head: WhenHead,
    then_block: Option<Spanned<Block>>,
    else_keyword: Option<SpanId>,
    statements: Vec<Spanned<Statement>>,
}

enum Body {
    Binding(Box<PendingBody>),
    Has(Box<PendingHas>),
    When(Box<PendingWhen>),
}

/// Statements are read in a function body and in the block of a `when` statement.
fn in_statements(bodies: &[Body]) -> bool {
    matches!(bodies.last(), Some(Body::Binding(_) | Body::When(_)))
}

impl Cursor<'_, '_> {
    pub fn parse_items(&mut self) -> Result<Vec<Spanned<Item>>, ParseFailure> {
        let mut items = Vec::new();
        let mut bodies: Vec<Body> = Vec::new();
        for _ in 0..=self.tokens.len() {
            self.options.checkpoint(self.current_span())?;
            self.skip_newlines();
            if self.current().is_none() {
                assert!(bodies.is_empty(), "layout normalization closes every body");
                return Ok(items);
            }
            let start = self.position;
            match self.parse_item_step(&mut items, &mut bodies) {
                Ok(()) => {}
                Err(ParseError::Abort(failure)) => return Err(failure),
                Err(ParseError::Syntax(diagnostic)) => {
                    self.record(diagnostic)?;
                    let span = self.recover_statement(start)?;
                    match bodies.last_mut() {
                        Some(Body::Binding(body)) => {
                            body.statements.push(Spanned::new(Statement::Error, span));
                        }
                        Some(Body::When(body)) => {
                            body.statements.push(Spanned::new(Statement::Error, span));
                        }
                        Some(Body::Has(body)) => {
                            body.members.push(Spanned::new(TypeMember::Error, span));
                        }
                        None => items.push(Spanned::new(Item::Error, span)),
                    }
                }
            }
        }
        unreachable!("each file step consumes a token or returns");
    }

    fn parse_item_step(
        &mut self,
        items: &mut Vec<Spanned<Item>>,
        bodies: &mut Vec<Body>,
    ) -> ParseResult<()> {
        match self.current() {
            Some(Token::Dedent) => {
                self.close_file_body(items, bodies)?;
            }
            Some(Token::Use) if bodies.is_empty() => {
                let import = self.parse_import()?;
                items.push(Spanned::new(
                    Item::Import(Box::new(import.value)),
                    import.span_id,
                ));
            }
            Some(Token::Pound) if bodies.is_empty() && self.peek_spelling(1) != Some("todo") => {
                // Commas and newlines separate directives alike: `#unrestricted, #wrap`
                // is the same two marker items as two lines.
                items.push(self.parse_directive()?);
                while self.current() == Some(Token::Comma) {
                    self.advance()?;
                    if self.current() != Some(Token::Pound) {
                        return Err(self.error("expected a directive after ','"));
                    }
                    items.push(self.parse_directive()?);
                }
                self.require_line_end()?;
            }
            Some(Token::Private) if bodies.is_empty() => {
                let span = self.advance()?.span_id;
                self.require_line_end()?;
                items.push(Spanned::new(Item::Private, span));
            }
            Some(Token::Return) if in_statements(bodies) => {
                let statement = self.parse_return()?;
                self.require_line_end()?;
                append_statement(items, bodies, statement);
            }
            Some(Token::Pound) if matches!(bodies.last(), Some(Body::Has(_))) => {
                match self.parse_attributed_member()? {
                    Attributed::Complete(binding) => {
                        self.require_line_end()?;
                        append_binding(items, bodies, binding);
                    }
                    Attributed::Body(body) => bodies.push(Body::Binding(Box::new(body))),
                }
            }
            _ if bodies.is_empty() && self.starts_type() => match self.parse_type()? {
                ParsedType::Complete(declaration) => {
                    items.push(Spanned::new(
                        Item::Type(Box::new(declaration.value)),
                        declaration.span_id,
                    ));
                }
                ParsedType::Body(body) => bodies.push(Body::Has(Box::new(body))),
            },
            _ if in_statements(bodies) && self.starts_assignment() => {
                let statement = self.parse_assignment()?;
                self.require_line_end()?;
                append_statement(items, bodies, statement);
            }
            _ if in_statements(bodies) && self.starts_destructure() => {
                let statement = self.parse_destructure()?;
                self.require_line_end()?;
                append_statement(items, bodies, statement);
            }
            _ if self.starts_binding()
                || matches!(bodies.last(), Some(Body::Has(_)))
                    && self.current() == Some(Token::Id) =>
            {
                self.parse_file_binding(items, bodies)?;
            }
            _ if matches!(bodies.last(), Some(Body::Has(_))) => {
                return Err(self.error("expected a simple named has member"));
            }
            Some(Token::When) if in_statements(bodies) && self.parse_when_statement(items, bodies)? => {}
            _ => {
                let expression = self.parse_expression()?;
                self.require_line_end()?;
                let span = expression.span_id;
                append_statement(
                    items,
                    bodies,
                    Spanned::new(Statement::Expression(expression), span),
                );
            }
        }
        Ok(())
    }

    /// `when condition then` followed by a statement body is a statement; the expression forms
    /// are left to the expression parser. Returns whether a statement was read.
    fn parse_when_statement(
        &mut self,
        items: &mut Vec<Spanned<Item>>,
        bodies: &mut Vec<Body>,
    ) -> ParseResult<bool> {
        let Some(head) = self.parse_when_head()? else {
            return Ok(false);
        };
        let pending = PendingWhen {
            head,
            then_block: None,
            else_keyword: None,
            statements: Vec::new(),
        };
        self.begin_when_block(items, bodies, pending)?;
        Ok(true)
    }

    /// The block after `then` or `else`: indented lines, or one statement on the same line.
    fn begin_when_block(
        &mut self,
        items: &mut Vec<Spanned<Item>>,
        bodies: &mut Vec<Body>,
        mut pending: PendingWhen,
    ) -> ParseResult<()> {
        if self.current() == Some(Token::Newline) {
            self.skip_newlines();
            if self.current() != Some(Token::Indent) {
                return Err(self.error("expected an indented block"));
            }
            self.advance()?;
            bodies.push(Body::When(Box::new(pending)));
            return Ok(());
        }
        let statement = if self.current() == Some(Token::Return) {
            self.parse_return()?
        } else if self.starts_assignment() {
            self.parse_assignment()?
        } else if self.starts_destructure() {
            self.parse_destructure()?
        } else {
            let expression = self.parse_expression()?;
            let span = expression.span_id;
            Spanned::new(Statement::Expression(expression), span)
        };
        self.require_line_end()?;
        pending.statements.push(statement);
        self.finish_when_block(items, bodies, pending)
    }

    /// A block has ended: the then-block may be followed by `else`, the else-block ends the
    /// statement.
    fn finish_when_block(
        &mut self,
        items: &mut Vec<Spanned<Item>>,
        bodies: &mut Vec<Body>,
        mut pending: PendingWhen,
    ) -> ParseResult<()> {
        let block = self.when_block(&mut pending.statements)?;
        if pending.then_block.is_some() {
            return self.complete_when(items, bodies, pending, Some(block));
        }
        pending.then_block = Some(block);
        let mut at = self.position;
        while self.tokens.get(at).map(|token| token.value) == Some(Token::Newline) {
            at += 1;
        }
        if self.tokens.get(at).map(|token| token.value) != Some(Token::Else) {
            return self.complete_when(items, bodies, pending, None);
        }
        self.position = at;
        pending.else_keyword = Some(self.advance()?.span_id);
        self.begin_when_block(items, bodies, pending)
    }

    fn when_block(
        &mut self,
        statements: &mut Vec<Spanned<Statement>>,
    ) -> ParseResult<Spanned<Block>> {
        let (Some(first), Some(last)) = (statements.first(), statements.last()) else {
            return Err(self.error("expected at least one statement in the block"));
        };
        let (start, end) = (first.span_id, last.span_id);
        Ok(self.node(
            Block {
                statements: std::mem::take(statements),
            },
            start,
            end,
        ))
    }

    fn complete_when(
        &mut self,
        items: &mut Vec<Spanned<Item>>,
        bodies: &mut Vec<Body>,
        pending: PendingWhen,
        else_block: Option<Spanned<Block>>,
    ) -> ParseResult<()> {
        let then_block = pending.then_block.expect("the then-block is read first");
        let end = else_block.as_ref().map_or(then_block.span_id, |block| block.span_id);
        let else_branch = else_block.map(|block| ElseBranch {
            else_keyword: pending.else_keyword.expect("an else-block follows `else`"),
            block,
        });
        let start = pending.head.when_keyword;
        let statement = self.node(pending.head.into_statement(then_block, else_branch), start, end);
        append_statement(items, bodies, statement);
        Ok(())
    }

    /// The top-level directives the language defines are `#entry`, `#wrap`, and
    /// `#unrestricted`; any other
    /// written `#name` is an unknown compiler query rather than retained metadata.
    fn parse_directive(&mut self) -> ParseResult<Spanned<Item>> {
        let hash = self.advance()?.span_id;
        if matches!(self.current(), None | Some(Token::Newline)) {
            return Err(self.error("expected a compiler query name after '#'"));
        }
        if self.peek_spelling(0) == Some("instruction") {
            let [_, _, text, close] = self.parse_directive_text("#instruction('mnemonic')")?;
            return Ok(self.node(Item::Instruction { text }, hash, close));
        }
        let item = match self.peek_spelling(0) {
            Some("entry") => Item::Entry,
            Some("wrap") => Item::Wrap,
            Some("load") => Item::Load,
            Some("store") => Item::Store,
            Some("unrestricted") => Item::Unrestricted,
            _ => {
                let name = self.peek_spelling(0).unwrap_or_default();
                return Err(ParseError::Syntax(
                    Diagnostic::new(
                        SyntaxCode::UnknownCompilerQuery,
                        format!("unknown compiler query `{name}`"),
                    )
                    .at_span(self.current_span()),
                ));
            }
        };
        let name = self.advance()?.span_id;
        Ok(self.node(item, hash, name))
    }

    /// The keyword and `('text')` of a directive: the spans of keyword, open paren, string
    /// and close paren.
    fn parse_directive_text(&mut self, form: &str) -> ParseResult<[ast::SpanId; 4]> {
        let keyword = self.advance()?.span_id;
        if self.current() != Some(Token::ParenOpen) {
            return Err(self.error(&format!("expected a string argument: {form}")));
        }
        let open = self.advance()?.span_id;
        if self.current() != Some(Token::String) {
            return Err(self.error(&format!("expected a string argument: {form}")));
        }
        let text = self.advance()?.span_id;
        if self.current() != Some(Token::ParenClose) {
            return Err(self.error(&format!("expected ')' to close {form}")));
        }
        let close = self.advance()?.span_id;
        Ok([keyword, open, text, close])
    }

    /// Attributes on their own lines (or comma-joined) before a `has` member: `#private`
    /// and `#register('x0')` before a field, `#infix('op')` before a method.
    fn parse_attributed_member(&mut self) -> ParseResult<Attributed> {
        let mut attributes = Vec::new();
        for _ in self.position..self.tokens.len() {
            if self.current() != Some(Token::Pound) {
                break;
            }
            let hash = self.advance()?.span_id;
            if self.current() == Some(Token::Private) {
                let private_keyword = self.advance()?.span_id;
                attributes.push(self.node(
                    FieldAttribute::Private {
                        hash,
                        private_keyword,
                    },
                    hash,
                    private_keyword,
                ));
            } else if self.peek_spelling(0) == Some("infix") {
                let keyword = self.advance()?.span_id;
                if self.current() != Some(Token::ParenOpen) {
                    return Err(self.error("#infix requires an operator string: #infix('+')"));
                }
                let open = self.advance()?.span_id;
                if self.current() != Some(Token::String) {
                    return Err(self.error("#infix requires an operator string: #infix('+')"));
                }
                let operator = self.advance()?.span_id;
                if self.current() != Some(Token::ParenClose) {
                    return Err(self.error("expected ')' after the #infix operator"));
                }
                let close = self.advance()?.span_id;
                attributes.push(self.node(
                    FieldAttribute::Infix {
                        hash,
                        keyword,
                        open,
                        operator,
                        close,
                    },
                    hash,
                    close,
                ));
            } else if self.peek_spelling(0) == Some("inline") {
                let keyword = self.advance()?.span_id;
                attributes.push(self.node(FieldAttribute::Inline { hash, keyword }, hash, keyword));
            } else if self.peek_spelling(0) == Some("register") {
                let [keyword, open, name, close] =
                    self.parse_directive_text("#register('name')")?;
                attributes.push(self.node(
                    FieldAttribute::Register {
                        hash,
                        keyword,
                        open,
                        name,
                        close,
                    },
                    hash,
                    close,
                ));
            } else {
                return Err(self.error(
                    "only #private, #inline, #register('name') and #infix('operator') attributes are supported",
                ));
            }
            // A comma separates attributes like a newline: `#private, #private`.
            if self.current() == Some(Token::Comma) {
                self.advance()?;
                if self.current() != Some(Token::Pound) {
                    return Err(self.error("expected an attribute after ','"));
                }
                continue;
            }
            if self.current() != Some(Token::Newline) {
                return Err(self.error("an attribute must end its line before a has member"));
            }
            self.skip_newlines();
        }
        let infix = attributes
            .iter()
            .any(|attribute| matches!(attribute.value, FieldAttribute::Infix { .. }));
        let register = attributes
            .iter()
            .any(|attribute| matches!(attribute.value, FieldAttribute::Register { .. }));
        if self.current() != Some(Token::Id) {
            return Err(self.error("an attribute must be followed by a has member"));
        }
        // A member with a parameter list is a method; `#private` marks a field or a method,
        // `#infix` a method, and `#register` a field.
        let method = self.starts_binding_parameters();
        if infix && register {
            return Err(self.error("#register marks a field and #infix a method, not both"));
        }
        if infix && !method {
            return Err(self.error("#infix requires a method with a parameter list"));
        }
        if register && method {
            return Err(self.error("#register marks a field, not a method or type"));
        }
        match self.parse_binding(BindingContext::Member)? {
            ParsedBinding::Complete(mut binding) => {
                binding.span_id = self.spans.merge(
                    attributes.first().expect("attribute starts with '#'").span_id,
                    binding.span_id,
                );
                binding.value.attributes = attributes;
                Ok(Attributed::Complete(binding))
            }
            ParsedBinding::Body(mut body) => {
                if !method {
                    return Err(
                        self.error("#private fields with indented bodies are not supported")
                    );
                }
                body.binding.attributes = attributes;
                Ok(Attributed::Body(body))
            }
        }
    }

    fn parse_file_binding(
        &mut self,
        items: &mut Vec<Spanned<Item>>,
        bodies: &mut Vec<Body>,
    ) -> ParseResult<()> {
        let context = if matches!(bodies.last(), Some(Body::Has(_))) {
            BindingContext::Member
        } else {
            BindingContext::Value
        };
        match self.parse_binding(context)? {
            ParsedBinding::Complete(binding) => {
                if matches!(bodies.last(), Some(Body::Has(_)))
                    && self.current() == Some(Token::Comma)
                {
                    self.advance()?;
                } else {
                    self.require_line_end()?;
                }
                append_binding(items, bodies, binding);
            }
            ParsedBinding::Body(body) => {
                bodies.push(Body::Binding(Box::new(body)));
            }
        }
        Ok(())
    }

    fn close_file_body(
        &mut self,
        items: &mut Vec<Spanned<Item>>,
        bodies: &mut Vec<Body>,
    ) -> ParseResult<()> {
        assert_eq!(self.current(), Some(Token::Dedent));
        let body = bodies
            .pop()
            .ok_or_else(|| self.error("unexpected dedent"))?;
        self.advance()?;
        match body {
            Body::Binding(body) => {
                let binding = self.close_body(*body)?;
                append_binding(items, bodies, binding);
            }
            Body::When(pending) => self.finish_when_block(items, bodies, *pending)?,
            Body::Has(body) => {
                assert!(bodies.is_empty(), "type bodies are top-level only");
                let declaration = self.close_has(*body);
                items.push(Spanned::new(
                    Item::Type(Box::new(declaration.value)),
                    declaration.span_id,
                ));
            }
        }
        Ok(())
    }

    pub fn parse_return(&mut self) -> ParseResult<Spanned<Statement>> {
        let start = self.advance()?.span_id;
        let mut end = start;
        let mut pending = crate::expr::returns::PendingReturnValue::default();
        if !matches!(
            self.current(),
            None | Some(Token::Newline | Token::Dedent | Token::Thus)
        ) {
            for _ in 0..=self.tokens.len() {
                let value = self.parse_return_part()?;
                end = value.expression.span_id;
                pending.push(value);
                if self.current() != Some(Token::Comma) {
                    break;
                }
                pending.comma(self.advance()?.span_id);
            }
        }
        let value = pending.finish(self)?.map(|value| value.expression);
        if self.current() == Some(Token::Thus) {
            return Err(self.error(
                "a claim is written in the header's `where` clause, not on a return",
            ));
        }
        Ok(self.node(
            Statement::Return(ReturnStatement { value }),
            start,
            end,
        ))
    }

    fn close_body(&mut self, mut body: PendingBody) -> ParseResult<Spanned<Binding>> {
        let Some(first) = body.statements.first() else {
            return Err(self.error("expected at least one body statement"));
        };
        let start = first.span_id;
        let end = body.statements.last().expect("nonempty body").span_id;
        let expression = self.node(
            Expr::Block(Block {
                statements: body.statements,
            }),
            start,
            end,
        );
        body.binding.value = Some(BindingValue {
            operator: body.operator,
            expression,
        });
        let start = body
            .binding
            .attributes
            .first()
            .map_or(body.binding.name.span_id, |attribute| attribute.span_id);
        Ok(self.node(body.binding, start, end))
    }

    fn recover_statement(&mut self, start: usize) -> Result<ast::SpanId, ParseFailure> {
        let first = self.tokens[start].span_id;
        if self.position == start && self.current() == Some(Token::Indent) {
            let end = self.recover_indented_region(first)?;
            return Ok(self.spans.merge(first, end));
        }
        let mut end = self.tokens[start..self.position]
            .iter()
            .rev()
            .find(|token| !is_layout(token.value))
            .map_or(first, |token| token.span_id);
        // A failed import/type can already have entered its own indentation
        // without registering a body frame. Consume that scope, not just its
        // first bad line; its remaining members must not become top-level items.
        let mut depth = 0usize;
        for token in &self.tokens[start..self.position] {
            match token.value {
                Token::Indent => depth += 1,
                Token::Dedent => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        if depth > 0 {
            end = self.recover_open_region(depth, end)?;
            return Ok(self.spans.merge(first, end));
        }
        for _ in self.position..self.tokens.len() {
            self.options.checkpoint(self.current_span())?;
            if matches!(self.current(), None | Some(Token::Newline | Token::Dedent)) {
                break;
            }
            let token = &self.tokens[self.position];
            if !is_layout(token.value) {
                end = token.span_id;
            }
            self.position += 1;
        }
        if self.position == start {
            self.position += 1;
        }
        let next = self.tokens[self.position..]
            .iter()
            .position(|token| token.value != Token::Newline)
            .map_or(self.tokens.len(), |offset| self.position + offset);
        if self
            .tokens
            .get(next)
            .is_some_and(|token| token.value == Token::Indent)
        {
            self.position = next;
            end = self.recover_indented_region(end)?;
        }
        Ok(self.spans.merge(first, end))
    }

    fn recover_indented_region(
        &mut self,
        end: ast::SpanId,
    ) -> Result<ast::SpanId, ParseFailure> {
        assert_eq!(self.current(), Some(Token::Indent));
        self.position += 1;
        self.recover_open_region(1, end)
    }

    fn recover_open_region(
        &mut self,
        mut depth: usize,
        mut end: ast::SpanId,
    ) -> Result<ast::SpanId, ParseFailure> {
        assert!(depth > 0);
        assert!(end.is_valid());
        for _ in self.position..self.tokens.len() {
            self.options.checkpoint(self.current_span())?;
            let token = &self.tokens[self.position];
            match token.value {
                Token::Indent => depth += 1,
                Token::Dedent => depth -= 1,
                _ if !is_layout(token.value) => end = token.span_id,
                _ => {}
            }
            self.position += 1;
            if depth == 0 {
                return Ok(end);
            }
        }
        unreachable!("normalized indentation is balanced");
    }
}

fn is_layout(token: Token) -> bool {
    matches!(token, Token::Newline | Token::Indent | Token::Dedent)
}

fn append_binding(items: &mut Vec<Spanned<Item>>, bodies: &mut [Body], binding: Spanned<Binding>) {
    let span = binding.span_id;
    let binding = Box::new(binding.value);
    match bodies.last_mut() {
        Some(Body::Binding(body)) => body
            .statements
            .push(Spanned::new(Statement::Binding(binding), span)),
        Some(Body::When(body)) => body
            .statements
            .push(Spanned::new(Statement::Binding(binding), span)),
        Some(Body::Has(body)) => body
            .members
            .push(Spanned::new(TypeMember::Binding(binding), span)),
        None => items.push(Spanned::new(Item::Binding(binding), span)),
    }
}

fn append_statement(
    items: &mut Vec<Spanned<Item>>,
    bodies: &mut [Body],
    statement: Spanned<Statement>,
) {
    match bodies.last_mut() {
        Some(Body::Binding(body)) => body.statements.push(statement),
        Some(Body::When(body)) => body.statements.push(statement),
        Some(Body::Has(_)) => unreachable!("has bodies contain members, not statements"),
        None => {
            let Statement::Expression(expression) = statement.value else {
                unreachable!("only expressions are top-level statements");
            };
            items.push(Spanned::new(
                Item::Expression(expression),
                statement.span_id,
            ));
        }
    }
}
