; Identifiers
(identifier) @variable

; Types
(type_identifier) @type
(type_variable) @type

; Type declarations
(declare_statement
  name: (type_identifier) @type)

; Parameters
(parameter
  name: (identifier) @variable.parameter)

; Bind statements (function/variable definitions)
(bind_statement
  name: (identifier) @function)

; Function calls
(call_expression
  function: (identifier) @function.call)

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
(method_definition
  (bind_statement
    name: (identifier) @function.method))

; Self
"self" @variable.builtin

; Literals
(number) @number
(string) @string
(float) @number

; Format strings
(format_string) @string
(escape_sequence) @string.escape
(interpolation) @string.special

; Comments
(line_comment) @comment
(doc_comment) @comment
(module_doc_comment) @comment

; Attributes
(attributes) @attribute
; Keywords
"use" @keyword
"as" @keyword
"private" @keyword
"has" @keyword.type
"is" @keyword.operator
"or" @keyword.operator
"and" @keyword.operator
"not" @keyword.operator
((identifier) @keyword (#eq? @keyword "add"))
"return" @keyword.return
"if" @keyword.conditional
"when" @keyword.conditional
"then" @keyword
"else" @keyword.conditional
"for" @keyword.repeat
"while" @keyword.repeat
"in" @keyword
"loop" @keyword.repeat
"extern" @keyword

; Operators
"+" @operator
"-" @operator
"*" @operator
"/" @operator
"%" @operator
"=" @operator
"/=" @operator
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

; Punctuation
"(" @punctuation.bracket
")" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
"{" @punctuation.bracket
"}" @punctuation.bracket
"." @punctuation.delimiter
"," @punctuation.delimiter
