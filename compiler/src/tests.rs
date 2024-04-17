// #[cfg(test)]
// mod parse {
//     use std::collections::HashMap;

//     use crate::{
//         expr::{
//             define::{DataDefiniton, Define, Function},
//             literal::Literal,
//             Call, Expr,
//         },
//         gin_type::GinType,
//         Ngin,
//     };

//     #[test]
//     fn comments() {
//         let mut runtime = Ngin::new();
//         let module = runtime.include("../examples/lang/comments.gin".to_string());

//         let call1 = Call::new(
//             "print".to_string(),
//             Some(Box::new(Expr::Literal(Literal::String("a".to_string())))),
//         );

//         let body: Vec<Expr> = vec![Expr::Call(call1.clone()), Expr::Call(call1)];

//         assert_eq!(*module.get_body(), body);
//     }

//     #[test]
//     fn assign() {
//         let mut runtime = Ngin::new();
//         let module = runtime.include("../examples/lang/assign.gin".to_string());

//         let func1 = Function::new(
//             String::from("a"),
//             vec![Expr::Literal(Literal::Number(1))],
//             GinType::Number,
//         );

//         let func2 = Function::new(
//             String::from("c"),
//             vec![Expr::Literal(Literal::String(String::from("hi")))],
//             GinType::String,
//         );

//         let body: Vec<Expr> = vec![
//             Expr::Define(Define::Function(func1)),
//             Expr::Define(Define::Function(func2)),
//         ];

//         assert_eq!(*module.get_body(), body);
//     }

//     #[test]
//     fn bool() {
//         let mut runtime = Ngin::new();
//         let module = runtime.include("../examples/lang/bool.gin".to_string());

//         let func = Function::new(
//             String::from("a"),
//             vec![Expr::Literal(Literal::Bool(true))],
//             GinType::Bool,
//         );

//         let body: Vec<Expr> = vec![Expr::Define(Define::Function(func))];

//         assert_eq!(*module.get_body(), body);
//     }

//     #[test]
//     fn fn_call_fn() {
//         let mut runtime = Ngin::new();
//         let module = runtime.include("../examples/lang/fnCallFn.gin".to_string());

//         let func1 = Function::new(
//             String::from("a"),
//             vec![Expr::Literal(Literal::Number(10))],
//             GinType::Number,
//         );
//         let call1 = Call::new("a".to_string(), None);

//         let arg = Some(Box::new(Expr::Call(call1)));
//         let call2 = Call::new("print".to_string(), arg);

//         let body: Vec<Expr> = vec![Expr::Define(Define::Function(func1)), Expr::Call(call2)];

//         assert_eq!(*module.get_body(), body);
//     }

//     #[test]
//     fn hello_world() {
//         let mut runtime = Ngin::new();
//         let module = runtime.include("../examples/helloWorld.gin".to_string());

//         let s = Literal::String("Hello world".to_string());

//         let arg = Some(Box::new(Expr::Literal(s)));

//         let call = Call::new("print".to_string(), arg);

//         let body: Vec<Expr> = vec![Expr::Call(call)];

//         assert_eq!(*module.get_body(), body);
//     }

//     #[test]
//     fn nested() {
//         let mut runtime = Ngin::new();
//         let module = runtime.include("../examples/lang/nested.gin".to_string());

//         let func3 = Function::new(
//             String::from("personName"),
//             vec![Expr::Literal(Literal::String(String::from("John")))],
//             GinType::String,
//         );

//         let func2 = Function::new(
//             String::from("handle"),
//             vec![
//                 Expr::Define(Define::Function(func3)),
//                 Expr::Call(Call::new("personName".to_string(), None)),
//             ],
//             GinType::String,
//         );

//         let func1 = Function::new(
//             String::from("do"),
//             vec![
//                 Expr::Define(Define::Function(func2)),
//                 Expr::Call(Call::new("handle".to_string(), None)),
//             ],
//             GinType::String,
//         );

//         let func4 = Function::new(
//             String::from("secondDo"),
//             vec![Expr::Literal(Literal::String(String::from("hello")))],
//             GinType::String,
//         );
//         let body: Vec<Expr> = vec![
//             Expr::Define(Define::Function(func1)),
//             Expr::Define(Define::Function(func4)),
//         ];

//         assert_eq!(*module.get_body(), body);
//     }

//     #[test]
//     fn point() {
//         let mut runtime = Ngin::new();
//         let module = runtime.include("../examples/lang/point.gin".to_string());

//         let mut data_definition = DataDefiniton::new("Point".to_string());

//         data_definition.insert("x".to_string(), GinType::Number);
//         data_definition.insert("y".to_string(), GinType::Number);

//         let body: Vec<Expr> = vec![Expr::Define(Define::Data(data_definition))];

//         assert_eq!(*module.get_body(), body);
//     }

//     #[test]
//     fn single_line_point() {
//         let mut runtime = Ngin::new();
//         let module = runtime.include("../examples/lang/singleLinePoint.gin".to_string());

//         let mut data_definition = DataDefiniton::new("Point".to_string());

//         data_definition.insert("x".to_string(), GinType::Number);
//         data_definition.insert("y".to_string(), GinType::Number);

//         let body: Vec<Expr> = vec![Expr::Define(Define::Data(data_definition))];

//         assert_eq!(*module.get_body(), body);
//     }

//     #[test]
//     fn return_obj() {
//         let mut runtime = Ngin::new();
//         let module = runtime.include("../examples/lang/returnObj.gin".to_string());

//         let mut object_hash = HashMap::new();
//         object_hash.insert(String::from("index"), GinType::Number);
//         object_hash.insert(String::from("length"), GinType::Number);

//         let object_type = GinType::Object(object_hash);

//         let mut object_literal_hash = HashMap::new();
//         object_literal_hash.insert(String::from("index"), Expr::Literal(Literal::Number(0)));
//         object_literal_hash.insert(String::from("length"), Expr::Literal(Literal::Number(256)));

//         let func2 = Function::new(
//             String::from("state"),
//             vec![Expr::Literal(Literal::Data(object_literal_hash))],
//             object_type.clone(),
//         );

//         let func1 = Function::new(
//             String::from("main"),
//             vec![
//                 Expr::Define(Define::Function(func2)),
//                 Expr::Call(Call::new("state".to_string(), None)),
//             ],
//             object_type,
//         );

//         let body: Vec<Expr> = vec![Expr::Define(Define::Function(func1))];

//         assert_eq!(*module.get_body(), body);
//     }

//     // #[test]
//     // fn if_then() {
//     //     let module = ast("../examples/ifThen.gin");

//     //     let body: Vec<Expr> = vec![Expr::Define(Define::Function(
//     //         String::from("people"),
//     //         vec![Expr::Literal(Literal::List(vec![
//     //             Expr::Literal(Literal::String(String::from("john"))),
//     //             Expr::Literal(Literal::String(String::from("jared"))),
//     //             Expr::Literal(Literal::String(String::from("joseph"))),
//     //         ]))],
//     //         GinType::List(vec![GinType::String]),
//     //     ))];

//     //     assert_eq!(module.body, body);
//     // }

//     // #[test]
//     // fn less_than() {
//     //     let module = ast("../examples/lessThan.gin");

//     //     let body: Vec<Expr> = vec![Expr::Define(Define::Function(
//     //         String::from("people"),
//     //         vec![Expr::Literal(Literal::List(vec![
//     //             Expr::Literal(Literal::String(String::from("john"))),
//     //             Expr::Literal(Literal::String(String::from("jared"))),
//     //             Expr::Literal(Literal::String(String::from("joseph"))),
//     //         ]))],
//     //         GinType::List(vec![GinType::String]),
//     //     ))];

//     //     assert_eq!(module.body, body);
//     // }

//     // #[test]
//     // fn list() {
//     //     let module = ast("../examples/list.gin");

//     //     let body: Vec<Expr> = vec![Expr::Define(Define::Function(
//     //         String::from("people"),
//     //         vec![Expr::Literal(Literal::List(vec![
//     //             Expr::Literal(Literal::String(String::from("john"))),
//     //             Expr::Literal(Literal::String(String::from("jared"))),
//     //             Expr::Literal(Literal::String(String::from("joseph"))),
//     //         ]))],
//     //         GinType::List(vec![GinType::String]),
//     //     ))];

//     //     assert_eq!(module.body, body);
//     // }
// }
