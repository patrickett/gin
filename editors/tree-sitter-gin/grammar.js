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

  externals: ($) => [$._newline, $.unterminated_string, $._error_recovery, $._import_newline],

  word: ($) => $.identifier,

  reserved: {
    global: ($) => [
      "private", "return", "loop", "ref", "mut",
      "own", "deref", "then", "to", "thus", "when", "else", "self", "for",
      "while", "use", "has", "and", "as", "if", "in", "is", "not", "or",
    ],
  },

  conflicts: ($) => [
    [$.sequence_literal, $.sequence_type],
    [$.named_result, $._type_hint],
    [$.decl_member_list],
    [$.proof_result_type, $.proof_proposition],
    [$.when_pattern_arm, $.when_is_arm],
    [$.variant],
    [$.is_condition, $.when_pattern_arm, $.when_is_arm],
    [$.is_condition, $.when_pattern_arm],
    [$._declare_type_value, $.variant],
    [$.intersection_type, $.variant],
    [$.argument_list, $.compile_time_query],
    [$.attribute_call, $.compile_time_query],
    [$.compile_time_query],
    [$._type_hint, $.parameter],
    [$._type_hint, $._expression],
    [$._type_hint, $.type_union],
    [$.unit_type, $.parameters, $.argument_list],
    [$.unassigned_binding, $._type_hint],
    [$.parameters, $.argument_list],
    [$.parameters, $.unit_type],
    [$.unit_type, $.argument_list],
    [$.parameter, $._expression],
    [$.tag, $._expression],
    [$.aggregate_set, $.buf_set, $.record_set, $._expression],
    [$.qualified_tag, $._expression],
    [$.tag, $.qualified_tag],
    [$.declaration_parameters, $._expression],

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

    use_statement: ($) => seq("use", sep1(",", $.module_import)),

    import_symbol: ($) =>
      seq(
        choice($.identifier, $.type_identifier),
        repeat(seq(".", choice($.identifier, $.type_identifier))),
        optional(seq("as", choice($.identifier, $.type_identifier))),
      ),

    module_import: ($) =>
      seq(
        choice($.path, $.string, $.type_identifier, "self"),
        optional(
          choice(
            prec.right(20, seq(
              choice(token.immediate(/[ \t]+/), $._import_newline),
              choice(
                seq("as", $.identifier),
                seq($.import_symbol,
                  repeat(seq(choice(",", $._import_newline), $.import_symbol))),
              ),
            )),
          ),
        ),
      ),

    _top_level_item: ($) =>
      choice(
        $.declare_statement,
        $.bind_statement,
        $.return_statement,
        $.unassigned_binding,
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

    declaration_parameters: ($) => seq(
      "(", optional(nlist($, choice(alias($.identifier, $.type_variable), $.declaration_parameter))), ")",
    ),

    declaration_parameter: ($) => seq(
      field("name", $.identifier),
      choice(
        seq("has", $.trait_composition),
        seq(field("type", $._type_hint), optional(seq(":", field("default", choice("?", $._expression))))),
        seq(":", field("default", $._expression)),
      ),
    ),

    trait_composition: ($) =>
      seq(field("trait", $.tag), repeat(seq("and", field("trait", $.tag)))),

    decl_member_list: ($) => nlist($, $._decl_member),

    qualified_member_name: ($) =>
      seq(field("trait", $.type_identifier), ".", field("name", $.identifier)),

    _decl_member: ($) =>
      choice($.record_signature, $.record_field, $.record_property),

    record_signature: ($) =>
      prec.right(
        3,
        seq(
          choice(field("name", $.identifier), $.qualified_member_name),
          $.parameters,
          optional(field("return_type", choice($._type_hint, $.named_results))),
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
      prec.right(choice(
        seq(field("operator", ":="), field("value", choice($._expression, $.bare_sequence))),
        seq(field("operator", ":"), field("body", choice($.block_body, $.return_statement, $._expression, $.bare_sequence))),
      )),

    block_body: ($) => seq($._newline, repeat($._statement), $.return_statement),

    _declare_type_value: ($) =>
      choice(
        $.unit_type,
        $.range_type,
        $.in_range_type,
        $.intersection_type,
        $.union_type,
        $.literal,
      ),

    range_type: ($) =>
      seq(optional("-"), $.number, "to", optional("-"), $.number),

    in_range_type: ($) =>
      seq("in", optional("-"), $.number, "to", optional("-"), $.number),

    union_type: ($) => seq($.variant, repeat1(seq("or", $.variant))),

    intersection_type: ($) => seq($.tag, repeat(seq("and", $.tag))),

    variant: ($) =>
      seq(choice(prec(3, seq($.type_identifier, $.parameters)), $.tag, $.literal), optional(seq("-", ">", $._type_hint))),

    tag: ($) =>
      choice(
        $.type_application,
        $.qualified_tag,
        $.reference_type,
        $.pointer_type,
        $.type_identifier,
        $.self_type,
      ),

    type_application: ($) =>
      prec(
        2,
        seq(
          choice($.type_identifier, $.self_type),
          "(",
          nlist($, choice($.tag, alias($.identifier, $.type_variable), $.literal, $.binary_expression, $.compile_time_query, $.call_expression, $.unary_expression)),
          ")",
        ),
      ),

    pointer_type: ($) => prec.right(seq("@", $._type_hint)),

    qualified_tag: ($) =>
      prec.left(seq($.type_identifier, repeat1(seq(".", $.type_identifier)))),

    reference_type: ($) =>
      prec.right(
        seq(
          field("modifier", choice("ref", "mut")),
          optional($.reference_group),
          field("type", $._type_hint),
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
          optional(field("return_type", choice($._type_hint, $.named_results))),
          field("operator", choice(":=", ":", $._rebind_operator)),
          field("value", $._bind_value),
        ),
      ),

    unassigned_binding: ($) =>
      seq(field("name", $.identifier), field("type", choice($.tag, $.sequence_type))),

    named_results: ($) => prec.right(choice($.named_result, seq(choice($.named_result, $._type_hint), repeat1(seq(",", choice($.named_result, $._type_hint)))))),
    named_result: ($) => seq(field("name", $.identifier), field("type", $._type_hint)),

    _type_hint: ($) => choice($.type_union, $.proof_result_type, $.unit_type, $.tag, $.sequence_type, alias($.identifier, $.type_variable)),

    unit_type: ($) => seq("(", ")"),

    type_union: ($) => prec.left(seq(
      choice($.tag, $.proof_result_type),
      repeat1(seq("or", choice($.tag, $.proof_result_type))),
    )),

    proof_result_type: ($) => prec(1, seq($.type_identifier, "thus", $.proof_proposition)),
    proof_proposition: ($) => choice(
      $.proof_relation,
      prec.left(2, seq($.proof_proposition, "and", $.proof_proposition)),
      prec.left(1, seq($.proof_proposition, "or", $.proof_proposition)),
      prec.right(3, seq("not", $.proof_proposition)),
      seq("(", $.proof_proposition, ")"),
    ),
    proof_relation: ($) => choice(
      seq($._proof_term, choice("=", "==", "/=", "<", "<=", ">", ">="), $._proof_term),
      seq($._proof_term, "in", $._proof_term, "to", $._proof_term),
    ),
    _proof_term: ($) => choice(
      $.identifier, $.type_identifier, $.number, "self", $.compile_time_query,
      $.proof_arithmetic, seq("(", $._proof_term, ")"),
      seq(alias("PowerOfTwo", $.type_identifier), "(", $._proof_term, ")"),
    ),
    proof_arithmetic: ($) => choice(
      prec.left(4, seq($._proof_term, choice("+", "-"), $._proof_term)),
      prec.left(5, seq($._proof_term, choice("*", "%"), $._proof_term)),
      prec.right(6, seq("-", $._proof_term)),
    ),

    _bind_value: ($) => choice($._expression, $.bare_sequence, $.return_statement, $.block_body),

    bare_sequence: ($) => prec.right(choice(
      seq($.named_argument, repeat(seq(",", choice($.named_argument, $._expression)))),
      seq($._expression, repeat1(seq(",", choice($.named_argument, $._expression)))),
    )),

    return_statement: ($) => prec.right(seq("return", optional(sep1(",", choice($.named_argument, $._expression))))),

    named_argument: ($) => seq(field("name", $.identifier), ":", field("value", $._expression)),

    parameters: ($) => prec(3, prec.dynamic(1, seq("(", optional(nlist($, $.parameter)), ")"))),

    parameter: ($) =>
      choice(
        seq(
          field("name", choice($.identifier, $.self_parameter)),
          optional(
            seq(
              field("modifier", choice("own", "ref", "mut")),
              optional($.reference_group),
            ),
          ),
          optional(
            choice(
              field("type", choice($.tag, $.sequence_type)),
              field("type", alias($.identifier, $.type_variable)),
              seq(":", field("default", $._expression)),
            ),
          ),
        ),
        field("type", choice($.tag, $.sequence_type)),
      ),

    self_parameter: ($) => prec(1, "self"),

    attributes: ($) => repeat1(seq("#", $._attribute_item, optional(","))),

    _attribute_item: ($) => choice($.identifier, $.attribute_call),

    attribute_call: ($) => prec.dynamic(1,
      seq(field("name", choice($.identifier, alias("target", $.identifier), alias("asm", $.identifier))), field("arguments", $.argument_list))),

    _statement: ($) =>
      choice($.bind_statement, $.unassigned_binding, $.aggregate_set, $.buf_set, $.record_set, $._expression),

    if_expression: ($) =>
      prec.right(
        seq(
          "if",
          field("condition", $._condition),
          repeat($._statement),
          $.return_statement,
        ),
      ),

    _condition: ($) => choice(
      $.is_condition, $.logical_condition, $.negated_condition, $.parenthesized_condition,
    ),

    is_condition: ($) => seq(field("subject", $._expression), "is", field("pattern", $.is_pattern)),
    logical_condition: ($) => choice(
      prec.left(2, seq($._condition, "and", $._condition)),
      prec.left(1, seq($._condition, "or", $._condition)),
    ),
    negated_condition: ($) => prec.right(3, seq("not", $._condition)),
    parenthesized_condition: ($) => seq("(", $._condition, ")"),

    when_expression: ($) => prec.right(choice(
      seq(
        "when", field("subject", $._expression),
        choice(seq("is", repeat1($.when_pattern_arm)), repeat1($.when_is_arm)),
        optional($.when_else_arm),
      ),
      prec.dynamic(-1, seq(
        "when", $._condition, "then", $._expression,
        repeat($.when_cond_arm), optional($.when_else_arm),
      )),
    )),

    when_pattern_arm: ($) =>
      seq(field("pattern", $.is_pattern), repeat(seq("or", $.literal)), "then", field("body", $._expression)),

    when_is_arm: ($) => seq("is", $.is_pattern, "then", $._expression),
    when_cond_arm: ($) =>
      prec.right(seq($._condition, "then", $._expression)),
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
        field("condition", $._condition),
        repeat($._statement),
        "loop",
      ),

    pattern: ($) => choice($.identifier, seq("(", $.identifier, ")")),

    is_pattern: ($) =>
      choice($.tag, $.identifier, $.literal, prec(1, $.list_pattern), $.unit_type),

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
        $.sequence_literal,
        $.compile_time_query,
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
        [3, "<"],
        [3, ">"],
        [3, "<="],
        [3, ">="],
        [1, "or"],
        [2, "and"],
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
      prec.left(
        2,
        seq(field("start", $._expression), "to", field("end", $._expression)),
      ),

    cast_expression: ($) =>
      prec.left(
        5,
        seq(
          field("value", $._expression),
          "as",
          field("type", $._type_hint),
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

    aggregate_set: ($) =>
      seq(
        field("base", $.identifier),
        ".",
        field("index", $.number),
        $._rebind_operator,
        field("value", $._expression),
      ),

    buf_set: ($) =>
      seq(
        field("base", $.identifier),
        ".",
        $.argument_list,
        $._rebind_operator,
        field("value", $._expression),
      ),

    record_set: ($) => seq(
      field("base", $.identifier), ".", field("field", $.identifier),
      $._rebind_operator, field("value", $._expression),
    ),

    _rebind_operator: ($) => choice(
      "::", seq(choice("+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"), ":"),
    ),

    unary_expression: ($) => choice(
      prec.right(6, seq(field("operator", "-"), field("operand", $._expression))),
      prec.right(0, seq(
        field("operator", choice("@", "*", "ref", "mut", "deref")),
        field("operand", $._expression),
      )),
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

    sequence_literal: ($) => seq(
      "[",
      optional(choice(
        seq(field("init", $._expression), ";", field("size", $._expression)),
        nlist($, choice($.named_argument, $._expression)),
      )),
      "]",
    ),

    sequence_type: ($) => seq("[", optional(choice(
      seq($._type_hint, ";", $._expression),
      nlist($, choice($.named_result, $._type_hint)),
    )), "]"),

    compile_time_query: ($) => seq("#", choice(
      seq("target", optional(seq(".", field("field", $.identifier)))),
      seq(choice("size", "alignment", "index_bits"), "(", $._type_hint, ")"),
      seq("register", "(", $._expression, ")"),
      seq("asm", "(", $._expression, repeat(seq(choice(",", $._newline), $.named_argument)), optional(","), ")"),
    )),

    parenthesized_expression: ($) => seq("(", $._expression, ")"),

    format_string: ($) =>
      seq(
        '"',
        repeat(choice($.interpolation, $.escape_sequence, $._string_content)),
        choice('"', $.unterminated_string),
      ),

    _string_content: ($) => token.immediate(prec(1, /[^"\\\n]+/)),
    interpolation: ($) => seq(token.immediate("\\("), $._expression, ")"),
    escape_sequence: ($) => token.immediate(seq("\\", /./)),

    path: ($) => sep1(".", $.identifier),

    literal: ($) => choice($.float, $.number, $.string),

    float: ($) => /[0-9](_?[0-9])*\.[0-9](_?[0-9])*/,
    number: ($) => choice(/[0-9](_?[0-9])*/, /0[xX][0-9a-fA-F](_?[0-9a-fA-F])*/),
    string: ($) => token(seq("'", /[^'\n]*/, "'")),

    identifier: ($) =>
      /[a-z_][a-zA-Z0-9_]*/,
    type_identifier: ($) => /[A-Z][a-zA-Z0-9]*/,
  },
});

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
