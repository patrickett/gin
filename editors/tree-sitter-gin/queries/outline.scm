(bind_statement
  name: (identifier) @name) @item

(declare_statement
  name: (type_identifier) @name) @item

(method_definition
  (bind_statement
    name: (identifier) @name)) @item

(use_statement) @item
