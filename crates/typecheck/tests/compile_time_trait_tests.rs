use ast::ConstValue;
use internment::Intern;
use parser::cursor::TokenCursor;
use std::sync::Arc;
use typecheck::compile_time_trait::{CompileTimeTraitRegistry, trait_field_for_ty};
use typecheck::transform::{TransformCtx, transform};
use typecheck::{FileId, TagId};

const REFLECTION_SOURCE: &str = "\
BigInt is in 0...18446744073709551615\n\
Int is in 0...4294967295\n\
\
Bool is True or False\n\
\
List(x) has pointer Pointer(x), length Int\n\
Pointer(x) is @x\n\
PointerSize is Int\n\
\
String is bytes List(Int)\nToString has to_string String\nHappy has value Bool\n\
\
Type is Primitive(width Int, signed Bool)\n     or Record(name String, fields List(NamedTy))\n     or Union(name String, variants List(VariantShape))\n     or Tuple(elems List(Type))\n     or Ptr(inner Type)\n     or Ref(inner Type, mutable Bool)\n     or Array(elem Type, size Int)\n     or Opaque(name String)\n\
\
NamedTy has name String, ty Type\nVariantShape has name String, fields List(NamedTy)\n\
#ReflectionType\nReflectionType is Type\n#ReflectionNamed\nReflectionNamed is NamedTy\n#ReflectionAnonymous\nReflectionAnonymous is Type\n#ReflectionBits\nReflectionBits is Primitive\n#ReflectionAddress\nReflectionAddress is Ptr\n#ReflectionProduct\nReflectionProduct is Record\n#ReflectionSum\nReflectionSum is Union\n#ReflectionArray\nReflectionArray is Array\n#ReflectionInteger\nReflectionInteger is Primitive\n#ReflectionRecord\nReflectionRecord is Record\n#ReflectionUnion\nReflectionUnion is Union\n#ReflectionTuple\nReflectionTuple is Tuple\n#ReflectionUnit\nReflectionUnit is Type\n#ReflectionSigned\nReflectionSigned is Bool\n#ReflectionUnsigned\nReflectionUnsigned is Bool\n#ReflectionRawPointer\nReflectionRawPointer is Ptr\n#ReflectionSafeReference\nReflectionSafeReference is Ref\n#ReflectionObserve\nReflectionObserve is Bool\n#ReflectionMutate\nReflectionMutate is Bool\n#ReflectionTypeArgument\nReflectionTypeArgument has arg Type\n#ReflectionConstArgument\nReflectionConstArgument has arg BigInt\n#ReflectionDeclarationKey\nReflectionDeclarationKey has value String\n#ReflectionRecursiveReference\nReflectionRecursiveReference has target String, depth Int\n#ReflectionOpaque\nReflectionOpaque is Opaque\n#ReflectionUnsupported\nReflectionUnsupported has reason String\n#ReflectionTrait\nReflectable has shape Type\n\
Node has next Node\n\
";

fn recursive_shape() -> ConstValue {
    let parse_ast = TokenCursor::parse_source(REFLECTION_SOURCE);
    let parse_ast_for_registry = Arc::new(parse_ast.clone());
    let typed = transform(
        &parse_ast,
        FileId(0),
        &TransformCtx::with_package_compile_time_arc(Arc::clone(&parse_ast_for_registry)),
    );

    let node_ty = &typed.tags[&TagId(Intern::from_ref("Node"))].resolved_ty;
    let registry = CompileTimeTraitRegistry::from_parse_ast(&parse_ast, parse_ast_for_registry);
    let discovered = registry.reflection_contract_with_issues(&typed);
    assert!(
        discovered.contract.is_some(),
        "reflection contract issues: {:?}",
        discovered.issues
    );

    trait_field_for_ty("Reflectable", "shape", node_ty, &typed, &registry)
        .expect("recursive type should materialize a Reflectable shape")
}

fn recursive_reference_entries(value: &ConstValue) -> usize {
    match value {
        ConstValue::Tag { name, args, .. } => {
            let here = usize::from(name.as_str() == "ReflectionRecursiveReference");
            here + args.iter().map(recursive_reference_entries).sum::<usize>()
        }
        ConstValue::Record { fields } => fields
            .iter()
            .map(|(_, value)| recursive_reference_entries(value))
            .sum(),
        ConstValue::List(items) => items.iter().map(recursive_reference_entries).sum(),
        _ => 0,
    }
}

fn recursive_reference_depth(value: &ConstValue) -> Option<i128> {
    match value {
        ConstValue::Tag { name, args, .. }
            if name.as_str() == "ReflectionRecursiveReference" && args.len() == 2 =>
        {
            match (&args[0], &args[1]) {
                (ConstValue::String(_), ConstValue::Int(depth)) => ast::integer::to_i128(*depth),
                _ => None,
            }
        }
        ConstValue::Tag { args, .. } => args.iter().find_map(recursive_reference_depth),
        ConstValue::Record { fields } => fields
            .iter()
            .find_map(|(_, value)| recursive_reference_depth(value)),
        ConstValue::List(items) => items.iter().find_map(recursive_reference_depth),
        _ => None,
    }
}

#[test]
fn recursive_reflection_shape_uses_recursion_guard() {
    let shape = recursive_shape();
    assert!(
        recursive_reference_entries(&shape) >= 1,
        "expected reflection materialization to contain a recursive marker"
    );
}

#[test]
fn recursive_reflection_type_materialization_is_finite() {
    let shape = recursive_shape();
    assert!(
        recursive_reference_entries(&shape) >= 1,
        "expected recursive materialization marker"
    );
    assert!(
        recursive_reference_depth(&shape).is_some(),
        "expected recursive marker to carry a depth payload"
    );
}
