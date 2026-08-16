//! Shared parameter parsing helpers used by both `parse_bind`'s param list and
//! `parse_declare`'s record/declare param list.
//!
//! The two call sites historically diverged: only the declare side accepted the
//! `id id` form (parameter name + lowercase type-variable, e.g. `start x` in
//! `Range[x] has start x, end x`. This module centralises the
//! "after-name" classification so both parsers stay in sync.

use ast::{Parameter, ParameterKind};
use internment::Intern;
use lexer::Token;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;

impl<'src, 't> TokenCursor<'src, 't> {
    /// Classify the tokens that may follow an already-consumed parameter `name`.
    ///
    /// When `for_declare` is true (type declaration context), an `id Tag` form
    /// with a capitalized tag produces [`ParameterKind::ValueParam`] instead of
    /// [`ParameterKind::Tagged`], signaling a const-value parameter.
    ///
    /// Recognises (in order):
    /// - `id Tag` / `id Tag(args)` → value param (declare) or tagged (bind)
    /// - `id id` → tagged type variable (if lowercase) or tagged (if uppercase)
    /// - `id ref|mut type` → tagged reference/mutable reference
    /// - `id : expr` → default value
    /// - bare `id` → generic parameter
    pub fn parse_param_after_name(
        &mut self,
        expr_parser: ExprFn,
        name: Intern<String>,
        name_span: ast::SpanId,
        for_declare: bool,
    ) -> Option<(Intern<String>, Parameter)> {
        // In type declarations, `id Tag` with a capitalized tag is a value parameter.
        if for_declare
            && matches!(self.peek(), Some(Token::Tag(_)))
            && let Some(sp) = self.parse_type_annotation(expr_parser)
        {
            if self.eat(&Token::Colon) && self.eat(&Token::Question) {
                return Some((
                    name,
                    Parameter::new(name_span, ParameterKind::Inferred { ty: Box::new(sp) }),
                ));
            }
            return Some((
                name,
                Parameter::new(name_span, ParameterKind::ValueParam { ty: Box::new(sp) }),
            ));
        }

        if let Some(sp) = self.parse_type_annotation(expr_parser) {
            return Some((
                name,
                Parameter::new(name_span, ParameterKind::Tagged(Box::new(sp))),
            ));
        }

        if matches!(self.peek(), Some(Token::Ref) | Some(Token::Mut)) {
            let sp = self.parse_type_expr(expr_parser)?;
            return Some((
                name,
                Parameter::new(name_span, ParameterKind::Tagged(Box::new(sp))),
            ));
        }

        if self.eat(&Token::Colon) {
            let expr = expr_parser(self);
            return Some((
                name,
                Parameter::new(name_span, ParameterKind::Default(Box::new(expr))),
            ));
        }

        Some((name, Parameter::new(name_span, ParameterKind::Generic)))
    }
}
