/**
 * @file Gin grammar for tree-sitter
 * @author Patrick Trickett <patrickett@protonmail.com>
 * @license MIT
 */

/// <reference types="tree-sitter-cli/dsl" />

module.exports = grammar({
  name: "gin",

  extras: ($) => [
    /[ \t\r]/,
    $.line_comment,
    $.doc_comment,
    $.module_doc_comment,
    $._newline,
  ],

  externals: ($) => [$._newline],

  word: ($) => $.identifier,

  conflicts: ($) => [
    [$.parameters, $.argument_list],
    [$.parameters, $.unit_type],
    [$.unit_type, $.argument_list],
    [$.parameter, $._expression],
    [$.tag, $._expression],
    [$.tuple_set, $.buf_set, $._expression],
    [$.qualified_tag, $._expression],
    [$.tag, $.qualified_tag, $._expression],
    [$.tag, $.qualified_tag],
    [$.declaration_parameters, $.type_application],
    [$.declaration_parameters, $._expression],

    [$.intersection_type],
    [$.path],
    [$.record_signature, $.record_field],
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

    use_statement: ($) => seq("use", sep1(",", $.module_import)),

    import_symbol: ($) =>
      seq(
        choice($.identifier, $.type_identifier),
        optional(seq("as", choice($.identifier, $.type_identifier))),
      ),

    module_import: ($) =>
      seq(
        choice($.path, $.string, $.type_identifier),
        optional(
          choice(
            seq(".", "(", nlist($, $.import_symbol), ")"),
            seq(".", $.import_symbol),
            seq("as", $.identifier),
          ),
        ),
      ),

    _top_level_item: ($) =>
      choice(
        $.declare_statement,
        $.bind_statement,
        $.return_statement,
        $._expression,
      ),

    declare_statement: ($) =>
      prec(
        10,
        seq(
          optional($.attributes),
          field("name", $.type_identifier),
          optional($.declaration_parameters),
          choice(
            seq(
              "has",
              choice(
                $.decl_member_list,
                seq(
                  $.trait_composition,
                  optional(seq($._newline, $.decl_member_list)),
                ),
              ),
            ),
            seq("is", $._declare_type_value),
          ),
        ),
      ),

    declaration_parameters: ($) =>
      choice(
        seq("(", optional(nlist($, alias($.identifier, $.type_variable))), ")"),
        seq(
          "(",
          alias($.identifier, $.type_variable),
          ",",
          nlist($, $.declaration_parameter),
          ")",
        ),
      ),

    declaration_parameter: ($) =>
      seq(
        field("name", $.identifier),
        field("type", $._type_hint),
        optional(seq(":", field("default", choice("?", $._expression)))),
      ),

    trait_composition: ($) =>
      seq(field("trait", $.tag), repeat(seq("and", field("trait", $.tag)))),

    decl_member_list: ($) => prec.right(nlist($, $._decl_member)),

    qualified_member_name: ($) =>
      seq(field("trait", $.type_identifier), ".", field("name", $.identifier)),

    _decl_member: ($) =>
      choice($.record_signature, $.record_field, $.record_property),

    record_signature: ($) =>
      prec.right(
        seq(
          choice(field("name", $.identifier), $.qualified_member_name),
          $.parameters,
          optional(field("return_type", $._type_hint)),
          optional($.member_default),
        ),
      ),

    record_field: ($) =>
      prec.right(
        2,
        seq(
          field("name", $.identifier),
          field("type", $._type_hint),
          optional($.field_default),
        ),
      ),

    record_property: ($) =>
      prec.right(
        seq(
          choice(field("name", $.identifier), $.qualified_member_name),
          optional($.field_default),
        ),
      ),

    field_default: ($) =>
      seq(field("operator", choice(":=", ":")), field("value", $._expression)),

    member_default: ($) =>
      choice(
        seq(field("operator", ":="), field("value", $._expression)),
        seq(field("operator", ":"), field("body", $.block_body)),
      ),

    block_body: ($) => seq(repeat($._statement), $.return_statement),

    _declare_type_value: ($) =>
      choice(
        $.unit_type,
        $.range_type,
        $.in_range_type,
        $.intersection_type,
        $.union_type,
      ),

    range_type: ($) =>
      seq(optional("-"), $.number, "...", optional("-"), $.number),

    in_range_type: ($) =>
      seq("in", optional("-"), $.number, "...", optional("-"), $.number),

    union_type: ($) => seq($.variant, repeat1(seq("or", $.variant))),

    intersection_type: ($) => seq($.tag, repeat(seq("and", $.tag))),

    variant: ($) =>
      choice(prec(3, seq($.type_identifier, $.parameters)), $.tag),

    tag: ($) =>
      choice(
        $.type_application,
        $.generic_tag,
        $.qualified_tag,
        $.reference_type,
        $.type_identifier,
        $.self_type,
      ),

    type_application: ($) =>
      prec(
        2,
        seq(
          choice($.type_identifier, $.self_type),
          "(",
          nlist($, choice($.tag, alias($.identifier, $.type_variable))),
          ")",
        ),
      ),

    type_parameters: ($) =>
      seq(
        "[",
        optional(nlist($, choice($.tag, alias($.identifier, $.type_variable)))),
        "]",
      ),

    generic_tag: ($) => prec(1, seq($.type_identifier, $.type_parameters)),

    qualified_tag: ($) =>
      prec.left(seq($.type_identifier, repeat1(seq(".", $.type_identifier)))),

    reference_type: ($) =>
      prec.right(
        seq(
          field("modifier", choice("ref", "mut")),
          optional($.reference_group),
          field("type", $.tag),
        ),
      ),

    reference_group: ($) =>
      seq(
        "{",
        field("root", $.identifier),
        repeat(
          seq(".", field("segment", choice($.identifier, $.type_identifier))),
        ),
        "}",
      ),

    bind_statement: ($) =>
      prec.right(
        1,
        seq(
          optional($.attributes),
          optional(field("binding_modifier", choice("ref", "mut"))),
          field("name", $.identifier),
          optional($.parameters),
          optional(field("return_type", $._type_hint)),
          field("operator", choice(":=", ":")),
          field("value", $._bind_value),
        ),
      ),

    _type_hint: ($) => choice($.type_union, $.unit_type, $.tag),

    unit_type: ($) => seq("(", ")"),

    type_union: ($) => prec.left(seq($.tag, repeat1(seq("or", $.tag)))),

    _bind_value: ($) => choice("extern", $._expression),

    return_statement: ($) => prec.right(seq("return", optional($._expression))),

    parameters: ($) => seq("(", optional(nlist($, $.parameter)), ")"),

    parameter: ($) =>
      choice(
        seq(
          optional(
            choice(
              seq(
                field("modifier", choice("ref", "mut")),
                optional($.reference_group),
              ),
              field("modifier", "eat"),
            ),
          ),
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

    attributes: ($) => repeat1(seq("#", $._attribute_item, optional(","))),

    _attribute_item: ($) => choice($.identifier, $.attribute_call),

    attribute_call: ($) =>
      seq(field("name", $.identifier), field("arguments", $.argument_list)),

    _statement: ($) =>
      choice($.bind_statement, $.tuple_set, $.buf_set, $._expression),

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

    for_expression: ($) =>
      seq(
        "for",
        field("pattern", $.pattern),
        "in",
        field("iterator", $._expression),
        repeat($._statement),
        "loop",
      ),

    while_expression: ($) =>
      seq(
        "while",
        field("condition", $._expression),
        repeat($._statement),
        "loop",
      ),

    pattern: ($) => choice($.identifier, $.tuple_pattern),

    tuple_pattern: ($) => seq("(", nlist($, $.identifier), ")"),

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

    tuple_is_pattern: ($) => seq("(", nlist($, $.is_pattern), ")"),

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
        $.self_type,
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
          field(
            "operator",
            choice("-", "@", "^", "*", "not", "ref", "mut", "eat"),
          ),
          field("operand", $._expression),
        ),
      ),

    call_expression: ($) =>
      prec(
        7,
        seq(
          field(
            "function",
            choice(
              $.member_expression,
              $.type_identifier,
              $.self_type,
              $.identifier,
            ),
          ),
          field("arguments", $.argument_list),
        ),
      ),

    argument_list: ($) => seq("(", optional(nlist($, $._expression)), ")"),

    self_type: ($) => "Self",

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
      seq("(", $._expression, ",", list($, $._expression), ")"),

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

    identifier: ($) =>
      /[a-z][a-zA-Z0-9]*(_[a-zA-Z0-9]+)*|_[a-zA-Z0-9]*(_[a-zA-Z0-9]+)*/,
    type_identifier: ($) => /[A-Z][a-zA-Z0-9]*/,
  },
});

function list($, rule) {
  return seq(rule, repeat(seq(",", rule)));
}

function nlist($, rule) {
  return seq(
    rule,
    repeat(seq(choice(",", $._newline), rule)),
    optional(choice(",", $._newline)),
  );
}

function sep1(sep, rule) {
  return seq(rule, repeat(seq(sep, rule)));
}
