use ast::{ImplBlock, MethodMap};
use lexer::Token;
use std::collections::HashMap;

use crate::cursor::TokenCursor;
use crate::expr::ExprFn;

impl<'src, 't> TokenCursor<'src, 't> {
    pub fn parse_impl_block(&mut self, expr_parser: ExprFn) -> Option<ImplBlock> {
        // Parse first Tag (type_name)
        let (type_name, type_name_span) = match self.peek()? {
            Token::Tag(name) => {
                let interned = self.intern(name);
                let span = self.peek_span()?;
                self.advance();
                (interned, span)
            }
            _ => return None,
        };

        // Expect dot
        if !self.eat(&Token::Dot) {
            self.error(
                "expected '.' after type name in impl block header",
                self.current_span(),
            );
            return None;
        }

        // Parse second Tag (trait_name)
        let trait_name = match self.peek()? {
            Token::Tag(name) => {
                let interned = self.intern(name);
                self.advance();
                interned
            }
            _ => {
                self.error(
                    "expected trait name after '.' in impl block header",
                    self.current_span(),
                );
                return None;
            }
        };

        // Expect '('
        if !self.eat(&Token::ParenOpen) {
            self.error("expected '(' after impl block header", self.current_span());
            return None;
        }

        // Optional indent
        self.eat(&Token::Indent);

        // Parse zero or more binds until ')'
        let mut methods: MethodMap = HashMap::new();
        loop {
            if self.is_at(&Token::ParenClose) {
                break;
            }
            if let Some(bind) = self.parse_bind(expr_parser) {
                methods.insert(bind.name, bind);
            } else {
                // If we can't parse a bind and we're not at ')', emit error and break
                if !self.is_at(&Token::ParenClose) {
                    self.error(
                        "expected bind or ')' in impl block body",
                        self.current_span(),
                    );
                }
                break;
            }
        }

        // Skip optional dedent, newlines
        self.eat(&Token::Dedent);

        // Expect ')'
        if !self.eat(&Token::ParenClose) {
            self.error("expected ')' to close impl block", self.current_span());
            return None;
        }

        Some(ImplBlock {
            type_name,
            type_name_span,
            trait_name,
            methods,
        })
    }
}
