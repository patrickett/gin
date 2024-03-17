#[cfg(test)]
mod parse {
    use std::collections::HashMap;

    use crate::{
        expr::{define::Define, literal::Literal, Expr},
        gin_type::GinType,
        Ngin,
    };

    #[test]
    fn comments() {
        let mut runtime = Ngin::new();
        let module = runtime.include("../examples/comments.gin".to_string());

        let body: Vec<Expr> = vec![
            Expr::Call(
                String::from("print"),
                Some(Box::new(Expr::Literal(Literal::String(String::from("a"))))),
            ),
            Expr::Call(
                String::from("print"),
                Some(Box::new(Expr::Literal(Literal::String(String::from("a"))))),
            ),
        ];

        assert_eq!(*module.get_body(), body);
    }

    #[test]
    fn assign() {
        let mut runtime = Ngin::new();
        let module = runtime.include("../examples/assign.gin".to_string());

        let body: Vec<Expr> = vec![
            Expr::Define(Define::Function(
                String::from("a"),
                vec![Expr::Literal(Literal::Number(1))],
                GinType::Number,
            )),
            Expr::Define(Define::Function(
                String::from("c"),
                vec![Expr::Literal(Literal::String(String::from("hi")))],
                GinType::String,
            )),
        ];

        assert_eq!(*module.get_body(), body);
    }

    #[test]
    fn bool() {
        let mut runtime = Ngin::new();
        let module = runtime.include("../examples/bool.gin".to_string());

        let body: Vec<Expr> = vec![Expr::Define(Define::Function(
            String::from("a"),
            vec![Expr::Literal(Literal::Bool(true))],
            GinType::Bool,
        ))];

        assert_eq!(*module.get_body(), body);
    }

    #[test]
    fn fn_call_fn() {
        let mut runtime = Ngin::new();
        let module = runtime.include("../examples/fnCallFn.gin".to_string());

        let body: Vec<Expr> = vec![
            Expr::Define(Define::Function(
                String::from("a"),
                vec![Expr::Literal(Literal::Number(10))],
                GinType::Number,
            )),
            Expr::Call(
                String::from("print"),
                Some(Box::new(Expr::Call(String::from("a"), None))),
            ),
        ];

        assert_eq!(*module.get_body(), body);
    }

    #[test]
    fn hello_world() {
        let mut runtime = Ngin::new();
        let module = runtime.include("../examples/helloWorld.gin".to_string());

        let body: Vec<Expr> = vec![Expr::Call(
            String::from("print"),
            Some(Box::new(Expr::Literal(Literal::String(String::from(
                "Hello world",
            ))))),
        )];

        assert_eq!(*module.get_body(), body);
    }

    #[test]
    fn nested() {
        let mut runtime = Ngin::new();
        let module = runtime.include("../examples/nested.gin".to_string());

        let body: Vec<Expr> = vec![
            Expr::Define(Define::Function(
                String::from("do"),
                vec![
                    Expr::Define(Define::Function(
                        String::from("handle"),
                        vec![
                            Expr::Define(Define::Function(
                                String::from("personName"),
                                vec![Expr::Literal(Literal::String(String::from("John")))],
                                GinType::String,
                            )),
                            Expr::Call(String::from("personName"), None),
                        ],
                        GinType::String,
                    )),
                    Expr::Call(String::from("handle"), None),
                ],
                GinType::String,
            )),
            Expr::Define(Define::Function(
                String::from("secondDo"),
                vec![Expr::Literal(Literal::String(String::from("hello")))],
                GinType::String,
            )),
        ];

        assert_eq!(*module.get_body(), body);
    }

    #[test]
    fn point() {
        let mut runtime = Ngin::new();
        let module = runtime.include("../examples/point.gin".to_string());

        let mut hash = HashMap::new();
        hash.insert(String::from("x"), GinType::Number);
        hash.insert(String::from("y"), GinType::Number);

        let body: Vec<Expr> = vec![Expr::Define(Define::Data(String::from("point"), hash))];

        assert_eq!(*module.get_body(), body);
    }

    #[test]
    fn single_line_point() {
        let mut runtime = Ngin::new();
        let module = runtime.include("../examples/singleLinePoint.gin".to_string());

        let mut hash = HashMap::new();
        hash.insert(String::from("x"), GinType::Number);
        hash.insert(String::from("y"), GinType::Number);

        let body: Vec<Expr> = vec![Expr::Define(Define::Data(String::from("point"), hash))];

        assert_eq!(*module.get_body(), body);
    }

    #[test]
    fn return_obj() {
        let mut runtime = Ngin::new();
        let module = runtime.include("../examples/returnObj.gin".to_string());

        let mut object_hash = HashMap::new();
        object_hash.insert(String::from("index"), GinType::Number);
        object_hash.insert(String::from("length"), GinType::Number);

        let object_type = GinType::Object(object_hash);

        let mut object_literal_hash = HashMap::new();
        object_literal_hash.insert(String::from("index"), Expr::Literal(Literal::Number(0)));
        object_literal_hash.insert(String::from("length"), Expr::Literal(Literal::Number(256)));

        let body: Vec<Expr> = vec![Expr::Define(Define::Function(
            String::from("main"),
            vec![
                Expr::Define(Define::Function(
                    String::from("state"),
                    vec![Expr::Literal(Literal::Object(object_literal_hash))],
                    object_type.clone(),
                )),
                Expr::Call(String::from("state"), None),
            ],
            object_type,
        ))];

        assert_eq!(*module.get_body(), body);
    }

    // #[test]
    // fn if_then() {
    //     let module = ast("../examples/ifThen.gin");

    //     let body: Vec<Expr> = vec![Expr::Define(Define::Function(
    //         String::from("people"),
    //         vec![Expr::Literal(Literal::List(vec![
    //             Expr::Literal(Literal::String(String::from("john"))),
    //             Expr::Literal(Literal::String(String::from("jared"))),
    //             Expr::Literal(Literal::String(String::from("joseph"))),
    //         ]))],
    //         GinType::List(vec![GinType::String]),
    //     ))];

    //     assert_eq!(module.body, body);
    // }

    // #[test]
    // fn less_than() {
    //     let module = ast("../examples/lessThan.gin");

    //     let body: Vec<Expr> = vec![Expr::Define(Define::Function(
    //         String::from("people"),
    //         vec![Expr::Literal(Literal::List(vec![
    //             Expr::Literal(Literal::String(String::from("john"))),
    //             Expr::Literal(Literal::String(String::from("jared"))),
    //             Expr::Literal(Literal::String(String::from("joseph"))),
    //         ]))],
    //         GinType::List(vec![GinType::String]),
    //     ))];

    //     assert_eq!(module.body, body);
    // }

    // #[test]
    // fn list() {
    //     let module = ast("../examples/list.gin");

    //     let body: Vec<Expr> = vec![Expr::Define(Define::Function(
    //         String::from("people"),
    //         vec![Expr::Literal(Literal::List(vec![
    //             Expr::Literal(Literal::String(String::from("john"))),
    //             Expr::Literal(Literal::String(String::from("jared"))),
    //             Expr::Literal(Literal::String(String::from("joseph"))),
    //         ]))],
    //         GinType::List(vec![GinType::String]),
    //     ))];

    //     assert_eq!(module.body, body);
    // }
}
