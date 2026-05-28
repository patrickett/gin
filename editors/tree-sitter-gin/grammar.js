/**
 * @file Gin grammar for tree-sitter
 * @author Patrick Trickett <patrickett@protonmail.com>
 * @license MIT
 */

/// <reference types="tree-sitter-cli/dsl" />

module.exports = grammar({
  name: "gin",

  extras: ($) => [/\s/, $.line_comment, $.doc_comment, $.module_doc_comment],

  word: ($) => $.identifier,

  conflicts: ($) => [
    [$.parameters, $.argument_list],
    [$.parameter, $._expression],
    [$.tag, $._expression],
    [$.tuple_set, $.buf_set, $._expression],
    [$.qualified_tag, $._expression],
    // Method definitions can have a bare-Tag, generic-Tag, or qualified-Tag
    // receiver; all overlap with the matching expression-position usage.
    [$.qualified_tag, $.impl_block, $.method_definition, $._expression],
    [$.when_expression, $.when_is_arm],
    [$.path],
  ],

  rules: {
    source_file: ($) =>
      seq(
        repeat($.use_statement),
        repeat($._top_level_item),
        optional($.private_section),
      ),

    // ── Comments ──────────────────────────────────────────

    doc_comment: ($) => token(prec(1, seq("---", /[^\n]*/))),
    module_doc_comment: ($) => token(prec(1, seq("--|", /[^\n]*/))),
    line_comment: ($) => token(prec(-1, seq("--", /[^\n]*/))),

    // ── Private section ──────────────────────────────────

    private_section: ($) => seq("private", repeat($._top_level_item)),

    // ── Use (imports) ────────────────────────────────────
    //
    // use http.web, crypto.hash
    // use './math' as math
    // use 'int'.(Int, Byte)
    // use dep.(Foo, Bar as Baz)

    use_statement: ($) => seq("use", sep1(",", $.module_import)),

    bundle_export: ($) =>
      seq(
        $.type_identifier,
        optional(seq("as", choice($.identifier, $.type_identifier))),
      ),

    module_import: ($) =>
      seq(
        choice($.path, $.string, $.type_identifier),
        optional(
          choice(
            seq(".", "(", sep1(",", $.bundle_export), ")"),
            seq("as", $.identifier),
          ),
        ),
      ),

    // ── Top-level items ──────────────────────────────────

    _top_level_item: ($) =>
      choice(
        $.declare_statement,
        $.impl_block,
        $.method_definition,
        $.bind_statement,
        $.return_statement,
        $._expression,
      ),

    // ── Declare ──────────────────────────────────────────
    //
    // Bool is True or False
    // Maybe[x] is Some(x) or None
    // Person has (name Str, age Int)
    // Int is -2147483648...2147483647
    // DiceThrow is in 1...6

    declare_statement: ($) =>
      seq(
        field("name", $.type_identifier),
        optional($.parameters),
        choice(
          seq("has", $._declare_record_value),
          seq("is", $._declare_type_value),
        ),
        repeat(seq("and", "is", optional("not"), $._declare_type_value)),
      ),

    _declare_record_value: ($) => choice($.tag, $.parameters),

    _declare_type_value: ($) =>
      choice($.range_type, $.in_range_type, $.union_type, $.tag),

    range_type: ($) =>
      seq(optional("-"), $.number, "...", optional("-"), $.number),

    in_range_type: ($) =>
      seq("in", optional("-"), $.number, "...", optional("-"), $.number),

    union_type: ($) => seq($.variant, repeat1(seq("or", $.variant))),

    variant: ($) => $.tag,

    // ── Tag (type reference) ─────────────────────────────
    //
    // Int                   nominal
    // Maybe[x]              generic
    // Bool.True             qualified

    tag: ($) => choice($.generic_tag, $.qualified_tag, $.type_identifier),

    type_parameters: ($) =>
      seq(
        "[",
        optional(
          sep1(",", choice($.tag, alias($.identifier, $.type_variable))),
        ),
        "]",
      ),

    generic_tag: ($) => prec(1, seq($.type_identifier, $.type_parameters)),

    qualified_tag: ($) =>
      prec.left(seq($.type_identifier, repeat1(seq(".", $.type_identifier)))),

    // ── Impl block ───────────────────────────────────────
    //
    // Args.Iterator (
    //     next:
    //         ...
    //     return result
    // )

    impl_block: ($) =>
      seq(
        field("type", $.type_identifier),
        ".",
        field("trait", $.type_identifier),
        "(",
        repeat(choice($.bind_statement, $.return_statement)),
        ")",
      ),

    // ── Method definition ────────────────────────────────
    //
    // Person.greet(self): ...

    // Person.greet(self): ...                   bare-Tag receiver
    // Range[x].new(start x, end x) Range[x]:    generic-Tag receiver
    // Maybe.Some.unwrap:                        qualified receiver
    method_definition: ($) =>
      seq(
        field(
          "receiver",
          choice($.generic_tag, $.qualified_tag, $.type_identifier),
        ),
        ".",
        $.bind_statement,
      ),

    // ── Bind ─────────────────────────────────────────────
    //
    // x: 42                       mutable bind
    // x := 42                     const bind
    // add(a Int, b Int) Int: a + b
    // main:
    //     print('hello')
    // return
    // syscall := extern

    bind_statement: ($) =>
      prec.right(
        1,
        seq(
          optional($.attributes),
          field("name", $.identifier),
          optional($.parameters),
          optional(field("return_type", $._type_hint)),
          field("operator", choice(":=", ":")),
          field("value", $._bind_value),
        ),
      ),

    _type_hint: ($) => $.tag,

    _bind_value: ($) => choice("extern", $._expression),

    return_statement: ($) => prec.right(seq("return", optional($._expression))),

    // ── Parameters ───────────────────────────────────────
    //
    // (a Int, b Int)
    // (x)
    // (p: 123)

    parameters: ($) => seq("(", optional(sep1(",", $.parameter)), ")"),

    parameter: ($) =>
      choice(
        seq(
          field("name", $.identifier),
          optional(
            choice(
              field("type", $.tag),
              field("type", alias($.identifier, $.type_variable)),
              seq(":", field("default", $._expression)),
            ),
          ),
        ),
        field("type", $.tag),
      ),

    // ── Attributes ───────────────────────────────────────
    //
    // #[test, inline]

    attributes: ($) => seq("#", "[", sep1(",", $._attribute_item), "]"),

    _attribute_item: ($) => choice("debug", "test", "inline"),

    // ── Statements ───────────────────────────────────────

    _statement: ($) =>
      choice($.bind_statement, $.tuple_set, $.buf_set, $._expression),

    // ── If ───────────────────────────────────────────────
    //
    // if val is Some(v)
    //     four: v + 1
    // return four

    if_expression: ($) =>
      prec.right(
        seq(
          "if",
          field("condition", $._expression),
          optional(seq("is", field("pattern", $.tag))),
          repeat($._statement),
          $.return_statement,
        ),
      ),

    // ── When ─────────────────────────────────────────────
    //
    // when value
    //     is Some(x) then x
    //     is None    then 0
    //
    // when n % 15 = 0 then print('FizzBuzz')
    //      n % 05 = 0 then print('Fizz')
    //      else print(n)

    when_expression: ($) =>
      prec.right(
        seq(
          "when",
          field("subject", $._expression),
          choice(
            seq(
              "then",
              $._expression,
              repeat($.when_cond_arm),
              optional($.when_else_arm),
            ),
            seq(
              "is",
              $.tag,
              "then",
              $._expression,
              repeat($.when_is_arm),
              optional($.when_else_arm),
            ),
            seq(repeat1($.when_is_arm), optional($.when_else_arm)),
          ),
        ),
      ),

    when_is_arm: ($) => seq("is", $.tag, "then", $._expression),
    when_cond_arm: ($) =>
      prec.right(seq($.binary_expression, "then", $._expression)),
    when_else_arm: ($) => seq("else", $._expression),

    // ── For loop ─────────────────────────────────────────
    //
    // for i in 1...50
    //     print(i)
    // loop

    for_expression: ($) =>
      seq(
        "for",
        field("pattern", $.pattern),
        "in",
        field("iterator", $._expression),
        repeat($._statement),
        "loop",
      ),

    // ── While loop ───────────────────────────────────────
    //
    // while x < 10
    //     x: x + 1
    // loop

    while_expression: ($) =>
      seq(
        "while",
        field("condition", $._expression),
        repeat($._statement),
        "loop",
      ),

    // ── Pattern ──────────────────────────────────────────

    pattern: ($) => choice($.identifier, $.tuple_pattern),

    tuple_pattern: ($) => seq("(", sep1(",", $.identifier), ")"),

    // ── Expressions ──────────────────────────────────────

    _expression: ($) =>
      choice(
        $.when_expression,
        $.if_expression,
        $.for_expression,
        $.while_expression,
        $.binary_expression,
        $.range_expression,
        $.cast_expression,
        $.member_expression,
        $.unary_expression,
        $.call_expression,
        $.self_expression,
        $.tuple_literal,
        $.tuple_alloc,
        $.parenthesized_expression,
        $.format_string,
        $.literal,
        $.type_identifier,
        $.identifier,
      ),

    // ── Binary expressions ───────────────────────────────

    binary_expression: ($) => {
      const table = [
        [3, "="],
        [3, "/="],
        [3, "<"],
        [3, ">"],
        [3, "<="],
        [3, ">="],
        [3, "&"],
        [3, "|"],
        [3, "^"],
        [3, "<<"],
        [3, ">>"],
        [4, "+"],
        [4, "-"],
        [4, "*"],
        [4, "/"],
        [4, "%"],
      ];
      return choice(
        ...table.map(([p, op]) =>
          prec.left(
            p,
            seq(
              field("left", $._expression),
              field("operator", op),
              field("right", $._expression),
            ),
          ),
        ),
      );
    },

    // ── Range ────────────────────────────────────────────

    range_expression: ($) =>
      prec.right(
        2,
        seq(field("start", $._expression), "...", field("end", $._expression)),
      ),

    // ── Cast ─────────────────────────────────────────────

    cast_expression: ($) =>
      prec.left(
        5,
        seq(
          field("value", $._expression),
          "as",
          field("type", $.type_identifier),
        ),
      ),

    // ── Member / tuple / buffer access ─────────────────

    member_expression: ($) =>
      prec.left(
        5,
        seq(
          field("base", $._expression),
          ".",
          field(
            "field",
            choice($.number, $.identifier, $.type_identifier, $.argument_list),
          ),
        ),
      ),

    tuple_set: ($) =>
      seq(
        field("base", $.identifier),
        ".",
        field("index", $.number),
        ":",
        field("value", $._expression),
      ),

    buf_set: ($) =>
      seq(
        field("base", $.identifier),
        ".",
        $.argument_list,
        ":",
        field("value", $._expression),
      ),

    // ── Unary ────────────────────────────────────────────

    unary_expression: ($) =>
      prec.right(
        6,
        seq(
          field("operator", choice("-", "@", "^", "*", "not")),
          field("operand", $._expression),
        ),
      ),

    // ── Call ─────────────────────────────────────────────

    call_expression: ($) =>
      prec(
        7,
        seq(
          field(
            "function",
            choice($.member_expression, $.type_identifier, $.identifier),
          ),
          field("arguments", $.argument_list),
        ),
      ),

    argument_list: ($) => seq("(", optional(sep1(",", $._expression)), ")"),

    // ── Self ─────────────────────────────────────────────

    self_expression: ($) =>
      prec.right(
        seq(
          "self",
          optional(
            seq(
              ".",
              field("member", $.identifier),
              optional(field("arguments", $.argument_list)),
            ),
          ),
        ),
      ),

    // ── Tuple literal ────────────────────────────────────

    tuple_literal: ($) =>
      seq(
        "(",
        $._expression,
        ",",
        sep1(",", $._expression),
        optional(","),
        ")",
      ),

    // ── Tuple alloc ──────────────────────────────────────

    tuple_alloc: ($) =>
      seq("(", field("init", $._expression), ";", field("size", $.number), ")"),

    // ── Parenthesized expression ─────────────────────────

    parenthesized_expression: ($) => seq("(", $._expression, ")"),

    // ── Format string ────────────────────────────────────

    format_string: ($) =>
      seq(
        '"',
        repeat(choice($.interpolation, $.escape_sequence, $._string_content)),
        '"',
      ),

    _string_content: ($) => token.immediate(prec(1, /[^"\\{}]+/)),
    interpolation: ($) => seq(token.immediate("{"), $._expression, "}"),
    escape_sequence: ($) => token.immediate(seq("\\", /./)),

    // ── Path (for imports) ───────────────────────────────

    path: ($) => sep1(".", $.identifier),

    // ── Literals ─────────────────────────────────────────

    literal: ($) => choice($.float, $.number, $.string),

    float: ($) => /\d+\.\d+/,
    number: ($) => choice(/\d+/, /0[xX][0-9a-fA-F]+/),
    string: ($) => token(seq("'", /[^'\n\r]*/, "'")),

    // ── Identifiers ──────────────────────────────────────

    identifier: ($) => /[a-z][a-z0-9]*(_[a-z0-9]+)*|_[a-z0-9]*(_[a-z0-9]+)*/,
    type_identifier: ($) => /[A-Z][a-zA-Z0-9]*/,
  },
});

function sep1(sep, rule) {
  return seq(rule, repeat(seq(sep, rule)));
}
