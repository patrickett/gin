use std::mem::{align_of, size_of};

use ast::{
    Bind, BindAttributes, BindValue, ConstValue, Declare, DeclareAttributes, DeclareValue, Expr,
    FileAst, HasFunction, HasMember, HasProperty, ParamSlot, Pattern, Ty, TyState, TypeExpr, Typed,
};

#[cfg(target_pointer_width = "64")]
#[test]
fn ast_layout_stays_within_current_ceilings() {
    macro_rules! check {
        ($ty:ty, $ceiling:expr) => {{
            let size = size_of::<$ty>();
            println!(
                "{:<32} size={:>4} align={}",
                stringify!($ty),
                size,
                align_of::<$ty>()
            );
            assert!(
                size <= $ceiling,
                "{} grew to {size} bytes (ceiling {} bytes)",
                stringify!($ty),
                $ceiling
            );
        }};
    }

    check!(Expr, 112);
    check!(Typed<Expr>, 312);
    check!(Ty, 104);
    check!(TyState, 112);
    check!(ConstValue, 88);
    check!(TypeExpr, 80);
    check!(Pattern, 80);
    check!(ParamSlot, 432);
    check!(Bind, 968);
    check!(BindValue, 40);
    check!(BindAttributes, 96);
    check!(DeclareAttributes, 160);
    check!(DeclareValue, 72);
    check!(Declare, 472);
    check!(HasMember, 360);
    check!(HasFunction, 360);
    check!(HasProperty, 208);
    check!(FileAst, 480);
}
