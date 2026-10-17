use diagnostic::code::FormatCode;
use std::ops::ControlFlow;

use ast::folder::{Visitor, walk_fn_call, walk_tag_call};
use ast::{
    BindValue, CallSyntax, Declare, DeclareValue, Expr, FnCall, HasMember, HasMemberBody, TagCall,
    Typed,
};
use diagnostic::{Category, Diagnostic};

use super::{Help, HelpContext};

impl HelpContext<'_> {
    pub(super) fn nested_construction_suggestions(&self) -> Vec<Help> {
        let mut visitor = NestedConstruction {
            context: self,
            helps: Vec::new(),
        };
        let _ = visitor.visit_file_ast(&self.parsed.ast);
        for declaration in self.parsed.ast.tags.values() {
            visitor.declaration(declaration);
        }
        visitor.helps
    }
}

struct NestedConstruction<'a, 'b> {
    context: &'a HelpContext<'b>,
    helps: Vec<Help>,
}

impl NestedConstruction<'_, '_> {
    fn declaration(&mut self, declaration: &Declare) {
        if let DeclareValue::Has(members) = &declaration.value {
            for member in members {
                let body = match member {
                    HasMember::Function(function) => &function.body,
                    HasMember::Property(property) => &property.body,
                };
                if let Some(HasMemberBody::Overrideable(body) | HasMemberBody::Final(body)) = body {
                    let _ = self.visit_bind_value(body);
                }
            }
        }
        for extension in &declaration.extensions {
            self.declaration(extension);
        }
    }

    fn argument(&mut self, argument: &Typed<Expr>) {
        let mut value = argument;
        loop {
            value = match &value.value {
                Expr::Bind(bind) => match &bind.value {
                    BindValue::Expr(value) => value,
                    _ => return,
                },
                Expr::Cast { expr, .. } => expr,
                _ => break,
            };
        }
        let construction = match &value.value {
            Expr::TagCall(_) => true,
            Expr::FnCall(call) => {
                call.path
                    .segments
                    .last()
                    .is_some_and(|name| name.as_str() == "new")
                    || (call.syntax == CallSyntax::Receiver && call.path.root.as_str() == "new")
            }
            _ => false,
        };
        if construction {
            self.helps.push(Help {
                // fmt-nested-construction
                diagnostic: Diagnostic::new(
                    FormatCode::NestedConstruction,
                    "construction is nested inside a call or constructor",
                )
                .with_category(Category::Help)
                .with_help("Create this value in a named binding before the enclosing call; use field shorthand when names match")
                .at_span(self.context.parsed.ast.span_table.get(value.span_id)),
                edits: Vec::new(),
            });
        }
    }
}

impl Visitor for NestedConstruction<'_, '_> {
    fn visit_fn_call(&mut self, call: &FnCall) -> ControlFlow<()> {
        if let Some(args) = &call.args {
            let skip = usize::from(call.syntax == CallSyntax::Receiver);
            for argument in args.iter().skip(skip) {
                self.argument(argument);
            }
        }
        walk_fn_call(self, call)
    }

    fn visit_tag_call(&mut self, call: &TagCall) -> ControlFlow<()> {
        for argument in &call.args {
            self.argument(argument);
        }
        walk_tag_call(self, call)
    }
}
