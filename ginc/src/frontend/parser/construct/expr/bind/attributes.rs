// TODO: bind attributes are not parsable yet and there is not even syntax for them yet

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct BindAttributes {
    /// Indicates if a test flag has been set for the bind.
    /// Will be run when we `begin test` and marked as PASS/FAIL
    test: bool,

    /// Tells the compiler that this bind should always be inlined
    inline_always: bool,
}
