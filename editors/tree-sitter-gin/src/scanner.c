#include <stdbool.h>
#include <tree_sitter/parser.h>

enum TokenType { NEWLINE };

void *tree_sitter_gin_external_scanner_create() { return NULL; }

void tree_sitter_gin_external_scanner_destroy(void *payload) {}

unsigned tree_sitter_gin_external_scanner_serialize(void *payload,
                                                    char *buffer) {
  return 0;
}

void tree_sitter_gin_external_scanner_deserialize(void *payload,
                                                  const char *buffer,
                                                  unsigned length) {}

bool tree_sitter_gin_external_scanner_scan(void *payload, TSLexer *lexer,
                                           const bool *valid_symbols) {
  if (!valid_symbols[NEWLINE])
    return false;

  if (lexer->lookahead == '\n' || lexer->lookahead == '\r') {
    if (lexer->lookahead == '\r') {
      lexer->advance(lexer, false);
      if (lexer->lookahead == '\n') {
        lexer->advance(lexer, false);
      }
    } else {
      lexer->advance(lexer, false);
    }

    lexer->result_symbol = NEWLINE;
    return true;
  }

  return false;
}
