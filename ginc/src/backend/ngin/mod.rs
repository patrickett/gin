/// ngin (EN-jin) is a interpreter for The Gin langauge that can be to act over
/// the source code at development time.
///
/// This is what enables compile time metaprogramming.

// Examples:
// - declared endpoint exists @endpoint("/api/hello")
//  will run code via the endpoint fn to make sure that /api/hello exists
//  or it will emit an error that the lsp/ compiler can expose

// Lisp style metaprogramming
