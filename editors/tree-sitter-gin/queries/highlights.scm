; Identifiers
(identifier) @variable
(member_expression field: (identifier) @property)

; Types
(type_identifier) @type
(type_variable) @type

; Type declarations
(declare_statement
  name: (type_identifier) @type)

; Parameters
(parameter
  name: (identifier) @variable.parameter)

(parameter
  name: (self_parameter) @variable.builtin)

; Parameter ownership is a semantic keyword even in an isolated hover signature.

(parameter modifier: "ref" @keyword)
(parameter modifier: "mut" @keyword)

; Bind statements (function/variable definitions)
(bind_statement
  name: (identifier) @function
  (parameters))

(record_signature
  name: (identifier) @function.method)

(record_field
  name: (identifier) @property)



; Function calls
(call_expression
  function: (identifier) @function.call)

(call_expression
  function: (member_expression
    field: (identifier) @function.method))


; Qualified type calls: Byte.new(255)
(call_expression
  function: (member_expression
    base: (type_identifier) @constructor
    field: (identifier) @function.method))

; Qualified type calls: Maybe.Some(42)
(call_expression
  function: (member_expression
    base: (type_identifier) @constructor
    field: (type_identifier) @function.method))

; Method definitions should highlight method names


; Self
"self" @variable.builtin

; Literals
(number) @number
(string) @string
(float) @number

; Format strings
(format_string) @string
(escape_sequence) @string.escape

; Comments
(line_comment) @comment
(doc_comment) @comment.documentation
(module_doc_comment) @comment.documentation

; Attributes
(attributes (identifier) @keyword.modifier)
(private_member_block (identifier) @keyword.modifier)
(attribute_call name: (identifier) @keyword.modifier)
(compile_time_query ["size" "alignment" "index_bits" "target" "emit" "todo"] @keyword.modifier)
; Keywords
"use" @keyword
"as" @keyword
"private" @keyword
[
    "has"
    "ref"
    "mut"
    "own"

] @keyword.modifier
"is" @keyword.operator
"or" @keyword.operator
"and" @keyword.operator
"not" @keyword.operator
"deref" @keyword.operator
"to" @keyword.operator

"return" @keyword.return
"when" @keyword.conditional
"then" @keyword
"else" @keyword.conditional
"for" @keyword.repeat
"while" @keyword.repeat
"do" @keyword.repeat
"in" @keyword
"domain" @keyword
"of" @keyword

; Operators
"+" @operator
"-" @operator
"*" @operator
"/" @operator
"%" @operator
"=" @operator
"<" @operator
">" @operator
"<=" @operator
">=" @operator
"&" @operator
"|" @operator
"^" @operator
"<<" @operator
">>" @operator
"..." @operator
":=" @operator
":" @operator
"::" @operator
"@" @operator

; Punctuation
"(" @punctuation.bracket
")" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
"{" @punctuation.bracket
"}" @punctuation.bracket
"." @punctuation.delimiter
"," @punctuation.delimiter

(self_type) @type.builtin
(named_result name: (identifier) @variable.parameter)
(named_argument name: (identifier) @property)
(record_set field: (identifier) @property)

"#" @keyword.modifier
";" @punctuation.delimiter
"?" @operator

((identifier) @keyword
  (#any-of? @keyword "break" "continue" "of"))

(interpolation "\\(" @punctuation.special ")" @punctuation.special)

"thus" @keyword
"==" @operator
"/=" @operator
