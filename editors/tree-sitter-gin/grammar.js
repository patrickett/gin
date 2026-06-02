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
    // [$.when_expression, $.when_is_arm],  -- removed: when_expression no longer references when_is_arm
    [$.path],
  ],

  rules: {
    source_file: ($) =>
      seq(
        repeat($.use_statement),
        repeat($._top_level_item),
        optional($.private_section),
      ),

    doc_comment: ($) => token(prec(1, seq("---", /[^\n]*/))),
    module_doc_comment: ($) => token(prec(1, seq("--|", /[^\n]*/))),
    line_comment: ($) => token(prec(-1, seq("--", /[^\n]*/))),

    private_section: ($) => seq("private", repeat($._top_level_item)),

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

    _top_level_item: ($) =>
      choice(
        $.declare_statement,
        $.blanket_impl,
        $.impl_block,
        $.method_definition,
        $.bind_statement,
        $.return_statement,
        $._expression,
      ),

    // Bool is True or False
    // Maybe[x] is Some(x) or None
    // Person has (name Str, age Int)
    // Int is -2147483648...2147483647
    // DiceThrow is in 1...6

    declare_statement: ($) =>
      prec(
        2,
        seq(
          field("name", $.type_identifier),
          optional($.parameters),
          choice(
            seq("has", $._declare_record_value),
            seq("is", $._declare_type_value),
          ),
          repeat(seq("and", "is", optional("not"), $._declare_type_value)),
        ),
      ),

    _declare_record_value: ($) => choice($.tag, $.record_fields),

    record_fields: ($) =>
      seq(
        "(",
        optional(
          seq(
            $._record_member,
            repeat(seq(",", $._record_member)),
            optional(","),
          ),
        ),
        ")",
      ),

    _record_member: ($) => choice($.record_signature, $.record_field),

    record_signature: ($) =>
      seq(
        field("name", $.identifier),
        $.parameters,
        optional(field("return_type", $._type_hint)),
      ),

    record_field: ($) => seq(field("name", $.identifier), field("type", $.tag)),

    _declare_type_value: ($) =>
      choice($.range_type, $.in_range_type, $.union_type, $.tag),

    range_type: ($) =>
      seq(optional("-"), $.number, "...", optional("-"), $.number),

    in_range_type: ($) =>
      seq("in", optional("-"), $.number, "...", optional("-"), $.number),

    union_type: ($) => seq($.variant, repeat1(seq("or", $.variant))),

    variant: ($) =>
      choice(prec(3, seq($.type_identifier, $.parameters)), $.tag),

    // Int                   nominal
    // Maybe[x]              generic
    // Bool.True             qualified

    tag: ($) =>
      choice(
        $.type_application,
        $.generic_tag,
        $.qualified_tag,
        $.type_identifier,
      ),

    type_application: ($) =>
      prec(
        2,
        seq(
          $.type_identifier,
          "(",
          sep1(",", choice($.tag, alias($.identifier, $.type_variable))),
          ")",
        ),
      ),

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

    // x has Sized(size: compute_size(x))
    // x has Copy(can_copy: is_copy(x))

    blanket_impl: ($) =>
      seq(
        field("type_var", $.identifier),
        "has",
        field("trait", $.type_identifier),
        "(",
        optional(sep1(",", $.trait_field)),
        ")",
      ),

    trait_field: ($) =>
      seq(field("name", $.identifier), ":", field("value", $._expression)),

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

    // x: 42                       mutable bind
    // x := 42                     const bind
    // add(a Int, b Int) Int: a + b
    // allocate(ref self, l Layout) Slice(Byte) or AllocError
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

    _type_hint: ($) => choice($.type_union, $.tag),

    type_union: ($) => prec.left(seq($.tag, repeat1(seq("or", $.tag)))),

    _bind_value: ($) => choice("extern", $._expression),

    return_statement: ($) => prec.right(seq("return", optional($._expression))),

    // (a Int, b Int)
    // (ref self, l Layout)
    // (x)
    // (p: 123)

    parameters: ($) => seq("(", optional(sep1(",", $.parameter)), ")"),

    parameter: ($) =>
      choice(
        seq(
          optional(field("modifier", "ref")),
          field("name", choice($.identifier, $.self_parameter)),
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

    self_parameter: ($) => prec(1, "self"),

    // #[test, inline]

    attributes: ($) => seq("#", "[", sep1(",", $._attribute_item), "]"),

    _attribute_item: ($) => choice("debug", "test", "inline"),

    _statement: ($) =>
      choice($.bind_statement, $.tuple_set, $.buf_set, $._expression),

    // if val is Some(v)
    //     four: v + 1
    // return four

    if_expression: ($) =>
      prec.right(
        seq(
          "if",
          field("condition", $._expression),
          optional(seq("is", field("pattern", $.is_pattern))),
          repeat($._statement),
          $.return_statement,
        ),
      ),

    // when scrutinee is
    //     pattern then consequent
    //     pattern then consequent
    //               else fallback
    //
    // when x > 0 then x
    //      y < 0 then y
    //            else 0

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
              $.when_pattern_arm,
              repeat($.when_pattern_arm),
              optional($.when_else_arm),
            ),
          ),
        ),
      ),

    when_pattern_arm: ($) =>
      seq(field("pattern", $.is_pattern), "then", field("body", $._expression)),

    when_is_arm: ($) => seq("is", $.is_pattern, "then", $._expression),
    when_cond_arm: ($) =>
      prec.right(seq($.binary_expression, "then", $._expression)),
    when_else_arm: ($) => seq("else", $._expression),

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

    pattern: ($) => choice($.identifier, $.tuple_pattern),

    tuple_pattern: ($) => seq("(", sep1(",", $.identifier), ")"),

    // Patterns after `is` in `if`/`when`. Lowercase identifiers are binders,
    // uppercase tags are variant/type patterns, and list patterns support
    // JS-style rest syntax: `[head, ...tail]`.
    is_pattern: ($) =>
      choice($.tag, $.identifier, $.list_pattern, $.tuple_is_pattern),

    list_pattern: ($) =>
      seq(
        "[",
        optional(
          seq(
            $.is_pattern,
            repeat(seq(",", $.is_pattern)),
            optional(seq(",", "...", $.is_pattern)),
          ),
        ),
        "]",
      ),

    tuple_is_pattern: ($) => seq("(", sep1(",", $.is_pattern), ")"),

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

    binary_expression: ($) => {
      const table = [
        [3, "="],
        [3, "/="],
        [3, "<"],
        [3, ">"],
        [3, "<="],
        [3, ">="],
        [3, "and"],
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

    range_expression: ($) =>
      prec.right(
        2,
        seq(field("start", $._expression), "...", field("end", $._expression)),
      ),

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

    unary_expression: ($) =>
      prec.right(
        6,
        seq(
          field("operator", choice("-", "@", "^", "*", "not")),
          field("operand", $._expression),
        ),
      ),

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

    tuple_literal: ($) =>
      seq(
        "(",
        $._expression,
        ",",
        sep1(",", $._expression),
        optional(","),
        ")",
      ),

    tuple_alloc: ($) =>
      seq("(", field("init", $._expression), ";", field("size", $.number), ")"),

    parenthesized_expression: ($) => seq("(", $._expression, ")"),

    format_string: ($) =>
      seq(
        '"',
        repeat(choice($.interpolation, $.escape_sequence, $._string_content)),
        '"',
      ),

    _string_content: ($) => token.immediate(prec(1, /[^"\\{}]+/)),
    interpolation: ($) => seq(token.immediate("{"), $._expression, "}"),
    escape_sequence: ($) => token.immediate(seq("\\", /./)),

    path: ($) => sep1(".", $.identifier),

    literal: ($) => choice($.float, $.number, $.string),

    float: ($) => /\d+\.\d+/,
    number: ($) => choice(/\d+/, /0[xX][0-9a-fA-F]+/),
    string: ($) => token(seq("'", /[^'\n\r]*/, "'")),

    identifier: ($) => /[a-z][a-z0-9]*(_[a-z0-9]+)*|_[a-z0-9]*(_[a-z0-9]+)*/,
    type_identifier: ($) => /[A-Z][a-zA-Z0-9]*/,
  },
});

function sep1(sep, rule) {
  return seq(rule, repeat(seq(sep, rule)));
}
