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

  externals: ($) => [$._newline, $.unterminated_string, $._error_recovery, $._import_newline, $._indent, $._dedent, $._private_indent, $._return_end],

  word: ($) => $.identifier,

  reserved: {
    global: ($) => [
      "private", "return", "ref", "mut",
      "own", "deref", "then", "to", "thus", "when", "case", "else", "self", "for",
      "while", "do", "use", "has", "and", "as", "in", "is", "not", "or",
    ],
  },

  conflicts: ($) => [
    [$._proof_term, $.self_expression],
    [$.proposition_condition, $._expression],
    [$._proof_term, $._expression_binding],
    [$._proof_term, $.literal],
    [$._proof_term, $._expression],
    [$.declaration_parameters, $.type_application],
    [$.named_result, $.type_union],
    [$._expression, $.record_set],
    [$.trait_composition],
    [$.type_application, $.argument_list],
    [$.type_application, $._expression],
    [$._type_hint, $.type_union, $._expression],
    [$.named_result, $._expression],
    [$.type_union, $._expression],
    [$.tag, $.qualified_tag, $._expression],
    [$.sequence_literal, $.sequence_type],
    [$.numeric_condition, $.binary_expression],
    [$.named_result, $._type_hint],
    [$.decl_member_list],
    [$.proof_result_type, $.proof_proposition],
    [$.variant],
    [$.variant, $.numbered_variant],
    [$._declare_type_value, $.variant],
    [$.intersection_type, $.variant, $.numbered_variant],
    [$.attribute_call, $.compile_time_query],
    [$.compile_time_query],
    [$._type_hint, $.parameter],
    [$._type_hint, $._expression],
    [$._type_hint, $.type_union],
    [$.unassigned_binding, $._type_hint],
    [$.unit_type, $.argument_list],
    [$.parameter, $._expression],
    [$.named_argument, $.parameter],
    [$.declaration_parameter, $.named_argument],
    [$.tag, $._expression],
    [$.aggregate_set, $.record_set, $._expression],
    [$.buf_set, $._expression],
    [$._expression_binding, $._expression],
    [$.qualified_tag, $._expression],
    [$.tag, $.qualified_tag],
    [$.range_parameter, $._expression],

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
        $.method_bind_statement,
        $.bind_statement,
        $.destructure_statement,
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
                  optional(alias($._indented_decl_member_list, $.decl_member_list)),
                ),
              ),
            ),
            seq("is", $._declare_type_value),
          ),
        ),
      ),

    declaration_parameters: ($) => seq(
      "(", optional(nlist($, choice($.range_parameter, alias($.identifier, $.type_variable), $.declaration_parameter))), ")",
    ),

    range_parameter: ($) => seq(field("start", $.identifier), "to", field("end", $.identifier)),

    declaration_parameter: ($) => seq(
      field("name", $.identifier),
      choice(
        seq("has", $.trait_composition),
        seq(field("type", $._type_hint), optional(seq(":", field("default", choice("?", $._expression))))),
        seq(":", field("default", $._expression)),
      ),
    ),

    trait_composition: ($) =>
      seq(field("trait", $.tag), repeat(seq("and", field("trait", $.tag))),
        optional(seq($._indent,
          repeat1(seq("and", field("trait", $.tag))), $._dedent))),

    decl_member_list: ($) => choice(
      seq($._decl_member, repeat(seq(",", $._decl_member)), optional(",")),
      $._indented_decl_member_list,
    ),

    _indented_decl_member_list: ($) => seq(
      $._indent,
      nlist($, choice($._decl_member, $.private_member_block)),
      $._dedent,
    ),

    private_member_block: ($) => seq(
      "#", alias("private", $.identifier), optional($.argument_list),
      $._private_indent, nlist($, $._decl_member), $._dedent,
    ),

    qualified_member_name: ($) =>
      seq(field("trait", $.type_identifier), ".", field("name", $.identifier)),

    _decl_member: ($) =>
      choice($.record_signature, $.record_field, $.record_property),

    record_signature: ($) =>
      prec.right(
        3,
        seq(
          optional($.attributes),
          choice(field("name", $.identifier), $.qualified_member_name),
          $.parameters,
          optional(field("return_type", choice($._type_hint, $.named_results))),
          optional($.refinement),
          optional($.member_default),
        ),
      ),

    record_field: ($) =>
      prec.right(
        2,
        seq(
          optional($.attributes),
          field("name", $.identifier),
          optional(field("modifier", "own")),
          field("type", $._type_hint),
          optional($.refinement),
          optional($.field_default),
        ),
      ),

    record_property: ($) =>
      prec.right(
        seq(
          optional($.attributes),
          choice(field("name", $.identifier), $.qualified_member_name),
          optional($.field_default),
        ),
      ),

    field_default: ($) =>
      seq(field("operator", choice(":=", ":")), field("value", $._expression)),

    member_default: ($) =>
      prec.right(choice(
        seq(field("operator", ":="), field("value", choice($._expression, $.bare_sequence, $.return_statement))),
        seq(field("operator", ":"), field("body", choice($.block_body, $.return_statement, $._expression, $.bare_sequence))),
      )),

    block_body: ($) => seq($._indent, repeat1($._statement), $._dedent),

    _declare_type_value: ($) =>
      choice(
        $.unit_type,
        $.range_type,
        $.in_range_type,
        $.intersection_type,
        $.union_type,
        $.numbered_variant,
        $.literal,
        $.self_expression,
      ),

    range_type: ($) =>
      seq(optional("-"), choice($.number, $.identifier), "to", optional("-"), choice($.number, $.identifier)),

    in_range_type: ($) =>
      seq("in", choice($.range_type, $.tag)),

    union_type: ($) => seq($.variant, repeat1(seq("or", $.variant))),

    intersection_type: ($) => seq($.tag, repeat(seq("and", $.tag))),

    variant: ($) =>
      choice(
        $.numbered_variant,
        seq(choice(prec(3, seq($.type_identifier, $.parameters)), $.tag, $.literal), optional(seq("-", ">", $._type_hint))),
      ),

    numbered_variant: ($) => prec.right(seq(
      choice(prec(3, seq($.type_identifier, $.parameters)), $.tag),
      optional(seq("-", ">", $._type_hint)),
      ":=", optional("-"), field("discriminant", $.number),
    )),

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
          nlist($, choice($.tag, alias($.identifier, $.type_variable), $.literal, $.binary_expression, $.compile_time_query, $.call_expression, $.unary_expression, $.domain_expression, $.range_expression, $.named_argument, $.unit_type, $.sequence_type, $.type_union, $.member_expression)),
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
          optional($.refinement),
          field("operator", choice(":=", ":", $._rebind_operator)),
          field("value", $._bind_value),
        ),
      ),

    method_bind_statement: ($) => seq(
      field("receiver", $.tag),
      ".",
      $.bind_statement,
    ),

    unassigned_binding: ($) => choice(
      seq(field("name", $.identifier), field("type", choice($.tag, $.sequence_type))),
      prec.right(seq(
        $.attributes,
        field("name", $.identifier),
        $.parameters,
        optional(field("return_type", choice($._type_hint, $.named_results))),
      )),
    ),

    named_results: ($) => prec.right(choice($.named_result, seq(choice($.named_result, $._type_hint), repeat1(seq(",", choice($.named_result, $._type_hint)))))),
    named_result: ($) => seq(field("name", $.identifier), field("type", $._type_hint)),

    proposition_type: ($) => seq("is", choice($.proof_proposition, $.predicate)),

    refinement: ($) => prec.right(seq("and", choice($.proof_proposition, $.predicate),
      repeat(seq("and", choice($.proof_proposition, $.predicate))))),

    predicate: ($) => choice(
      seq(choice("=", "/=", "<", "<=", ">", ">=", seq("not", "=")), $._proof_term),
      seq("*", $.proof_proposition),
    ),

    integer_domain_type: ($) => seq("is", "in", choice($.tag, $.range_type)),

    _type_hint: ($) => choice($.proposition_type, $.integer_domain_type, $.in_range_type, $.type_union, $.proof_result_type, $.unit_type, $.tag, $.sequence_type, alias($.identifier, $.type_variable)),

    unit_type: ($) => seq("(", ")"),

    type_union: ($) => prec.left(seq(
      choice($.tag, alias($.identifier, $.type_variable), $.proof_result_type, $.named_result),
      repeat1(seq("or", choice($.tag, alias($.identifier, $.type_variable), $.proof_result_type, $.named_result))),
    )),

    proof_result_type: ($) => prec(1, seq($.tag, "thus", $.proof_proposition)),
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
      $.proof_member,
      $.proof_arithmetic, seq("(", $._proof_term, ")"),
      seq(alias("PowerOfTwo", $.type_identifier), "(", $._proof_term, ")"),
    ),
    proof_member: ($) => prec(1, seq(choice($.identifier, "self"), repeat1(seq(".", $.identifier)))),
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

    return_statement: ($) => prec.right(seq("return", choice(
      $._return_end,
      seq(optional(sep1(",", choice($.named_argument, $._expression))), optional(seq("thus", $.proof_proposition))),
    ))),

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
              field("type", choice($.proposition_type, $.integer_domain_type, $.in_range_type, $.tag, $.sequence_type)),
              field("type", alias($.identifier, $.type_variable)),
              seq("has", $.trait_composition),
              seq(":", field("default", $._expression)),
            ),
          ),
          optional($.refinement),
        ),
        field("type", choice($.proposition_type, $.integer_domain_type, $.tag, $.sequence_type)),
      ),

    self_parameter: ($) => prec(1, "self"),

    attributes: ($) => repeat1(seq("#", $._attribute_item, optional(","))),

    _attribute_item: ($) => choice($.identifier, alias("private", $.identifier), $.attribute_call),

    attribute_call: ($) => prec.dynamic(1,
      seq(field("name", choice($.identifier, alias("private", $.identifier), alias("target", $.identifier))), field("arguments", $.argument_list))),

    _statement: ($) =>
      choice($.use_statement, $.bind_statement, $.destructure_statement, $.unassigned_binding, $.aggregate_set, $.buf_set, $.record_set, $.return_statement, $._expression),

    destructure_statement: ($) => seq(
      field("type", choice($.type_identifier, $.self_type)),
      field("fields", $.argument_list),
      ":=",
      field("value", $._expression),
    ),

    _condition: ($) => choice(
      $.proposition_condition, $.numeric_condition, $.membership_condition, $.is_condition, $.logical_condition, $.negated_condition, $.parenthesized_condition,
    ),

    proposition_condition: ($) => prec.dynamic(1, choice($.identifier, $.call_expression, $.member_expression)),

    numeric_condition: ($) => prec.left(3, seq(
      field("left", $._expression),
      field("operator", choice("=", "<", "<=", ">", ">=")),
      field("right", $._expression),
    )),
    membership_condition: ($) => seq(field("subject", $._expression), "in", field("domain", $._expression)),
    is_condition: ($) => seq(field("subject", $._expression), "is", field("pattern", $.is_pattern)),
    logical_condition: ($) => choice(
      prec.left(2, seq($._condition, "and", $._condition)),
      prec.left(1, seq($._condition, "or", $._condition)),
    ),
    negated_condition: ($) => prec.right(3, seq("not", $._condition)),
    parenthesized_condition: ($) => seq("(", $._condition, ")"),

    when_expression: ($) => prec.right(choice(
      seq("when", $._condition, "then", $._when_body, optional($.when_else_arm)),
      seq("when", $._condition, $._indent,
        "then", $._when_body, optional($.when_else_arm), $._dedent),
      seq("when", optional(field("subject", $._expression)), $._indent,
        repeat1(choice($.when_cond_arm, $.when_comparison_arm)), optional($.when_else_arm), $._dedent),
      seq("when", field("subject", $._expression), "is", $._indent,
        repeat1($.when_pattern_arm), optional($.when_else_arm), $._dedent),
    )),
    _when_body: ($) => choice($._expression, $.return_statement, $.block_body),
    when_guard: ($) => seq($._indent, repeat1(seq("and", $._condition)), $._dedent),
    when_pattern_arm: ($) => seq("case", field("pattern", $.is_pattern),
      repeat(seq("or", $.is_pattern)), optional(seq("and", $._condition)), optional($.when_guard),
      "then", field("body", $._when_body)),
    when_cond_arm: ($) => prec.right(seq("case", $._condition, optional($.when_guard), "then", $._when_body)),
    when_comparison_arm: ($) => seq("case", choice("=", "<", "<=", ">", ">="), $._expression,
      optional(seq("and", $._condition)), optional($.when_guard), "then", $._when_body),
    when_else_arm: ($) => seq("else", $._when_body),

    for_expression: ($) =>
      seq(
        "for",
        field("pattern", $.pattern),
        "in",
        field("iterator", $._expression),
        $._indent,
        repeat1($._statement),
        $._dedent,
      ),

    while_expression: ($) =>
      seq(
        "while",
        field("condition", $._condition),
        "do",
        $._indent,
        repeat1($._statement),
        $._dedent,
      ),

    pattern: ($) => choice($.identifier, seq("(", $.identifier, ")")),

    is_pattern: ($) =>
      choice($.tag, $.identifier, $.literal, $.in_range_type, prec(1, $.list_pattern), $.unit_type),

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
        $.for_expression,
        $.while_expression,
        $.binary_expression,
        $.range_expression,
        $.domain_expression,
        $.cast_expression,
        $.member_expression,
        $.index_expression,
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

    domain_expression: ($) => prec.right(6, seq("domain", "of", field("operand", $._expression))),

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
        7,
        seq(
          field("base", $._expression),
          ".",
          field(
            "field",
            choice($.number, $.identifier, $.type_identifier),
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

    index_expression: ($) => prec.left(7, seq(
      field("base", $._expression),
      ".", "[",
      field("index", $._expression),
      optional(seq(";", field("count", $._expression))),
      "]",
    )),

    buf_set: ($) => seq(
        $.index_expression,
        $._rebind_operator,
        field("value", $._expression),
    ),

    record_set: ($) => seq(
      field("base", choice($.identifier, $.self_expression, $.member_expression, $.index_expression, $.call_expression)), ".", field("field", $.identifier),
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

    argument_list: ($) => seq("(", optional(nlist($, choice($.named_argument, $._expression))), ")"),

    self_type: ($) => "Self",

    self_expression: ($) => "self",

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
      "todo",
      seq("target", optional(seq(".", field("field", $.identifier)))),
      seq(choice("size", "alignment", "index_bits"), "(", $._type_hint, ")"),
      seq("emit", "(", field("code", $._expression), ",", field("arguments", $._expression), ")"),
    )),

    parenthesized_expression: ($) => seq("(", choice(
      $._expression,
      alias($._expression_binding, $.bind_statement),
    ), ")"),

    _expression_binding: ($) => seq(
      field("name", $.identifier),
      optional(field("return_type", $._type_hint)),
      field("operator", choice(":", ":=")),
      field("value", $._expression),
    ),

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
