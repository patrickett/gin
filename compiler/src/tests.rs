#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use crate::{
        ast,
        parse::{Expr, FnArg, GinType, Module},
        token::Literal,
    };

    #[test]
    fn assign() {
        let module = ast("../examples/assign.gin");

        let body: Vec<Expr> = vec![
            Expr::FunctionDefinition(
                String::from("a"),
                vec![Expr::Literal(Literal::Number(1))],
                GinType::Number,
            ),
            Expr::FunctionDefinition(
                String::from("c"),
                vec![Expr::Literal(Literal::String(String::from("hi")))],
                GinType::String,
            ),
        ];

        assert_eq!(module.body, body);
    }

    #[test]
    fn bool() {
        let module = ast("../examples/bool.gin");

        let body: Vec<Expr> = vec![Expr::FunctionDefinition(
            String::from("a"),
            vec![Expr::Literal(Literal::Bool(true))],
            GinType::Bool,
        )];

        assert_eq!(module.body, body);
    }

    #[test]
    fn fn_call_fn() {
        let module = ast("../examples/fnCallFn.gin");

        let body: Vec<Expr> = vec![
            Expr::FunctionDefinition(
                String::from("a"),
                vec![Expr::Literal(Literal::Number(10))],
                GinType::Number,
            ),
            Expr::FunctionCall(String::from("print"), Some(FnArg::Id(String::from("a")))),
        ];

        assert_eq!(module.body, body);
    }

    #[test]
    fn hello_world() {
        let module = ast("../examples/helloWorld.gin");

        let body: Vec<Expr> = vec![Expr::FunctionCall(
            String::from("print"),
            Some(FnArg::String(String::from("Hello world"))),
        )];

        assert_eq!(module.body, body);
    }

    #[test]
    fn nested() {
        let module = ast("../examples/nested.gin");

        let body: Vec<Expr> = vec![
            Expr::FunctionDefinition(
                String::from("do"),
                vec![
                    Expr::FunctionDefinition(
                        String::from("handle"),
                        vec![
                            Expr::FunctionDefinition(
                                String::from("personName"),
                                vec![Expr::Literal(Literal::String(String::from("John")))],
                                GinType::String,
                            ),
                            Expr::FunctionCall(String::from("personName"), None),
                        ],
                        GinType::String,
                    ),
                    Expr::FunctionCall(String::from("handle"), None),
                ],
                GinType::String,
            ),
            Expr::FunctionDefinition(
                String::from("secondDo"),
                vec![Expr::Literal(Literal::String(String::from("hello")))],
                GinType::String,
            ),
        ];

        assert_eq!(module.body, body);
    }

    #[test]
    fn point() {
        let module = ast("../examples/point.gin");

        let mut hash = HashMap::new();
        hash.insert(String::from("x"), GinType::Number);
        hash.insert(String::from("y"), GinType::Number);

        let body: Vec<Expr> = vec![Expr::ObjectDefinition(String::from("point"), hash)];

        assert_eq!(module.body, body);
    }

    #[test]
    fn return_obj() {
        let module = ast("../examples/returnObj.gin");

        let mut object_hash = HashMap::new();
        object_hash.insert(String::from("index"), GinType::Number);
        object_hash.insert(String::from("length"), GinType::Number);

        let object_type = GinType::Object(object_hash);

        let mut object_literal_hash = HashMap::new();
        object_literal_hash.insert(String::from("index"), Expr::Literal(Literal::Number(0)));
        object_literal_hash.insert(String::from("length"), Expr::Literal(Literal::Number(256)));

        let body: Vec<Expr> = vec![Expr::FunctionDefinition(
            String::from("main"),
            vec![
                Expr::FunctionDefinition(
                    String::from("state"),
                    vec![Expr::ObjectLiteral(object_literal_hash)],
                    object_type.clone(),
                ),
                Expr::FunctionCall(String::from("state"), None),
            ],
            object_type,
        )];

        assert_eq!(module.body, body);
    }
}
