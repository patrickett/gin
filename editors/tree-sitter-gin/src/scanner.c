#include <stdbool.h>
#include <tree_sitter/parser.h>

enum TokenType { NEWLINE, UNTERMINATED_STRING, ERROR_RECOVERY, IMPORT_NEWLINE };

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
  if (valid_symbols[ERROR_RECOVERY])
    return false;

  if (valid_symbols[UNTERMINATED_STRING] &&
      (lexer->lookahead == '\n' || lexer->eof(lexer))) {
    lexer->result_symbol = UNTERMINATED_STRING;
    return true;
  }

  if (!valid_symbols[NEWLINE] && !valid_symbols[IMPORT_NEWLINE])
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

    lexer->mark_end(lexer);
    if (valid_symbols[IMPORT_NEWLINE]) {
      unsigned indent = 0;
      while (lexer->lookahead == ' ' || lexer->lookahead == '\t') {
        indent++;
        lexer->advance(lexer, false);
      }
      if (indent > 0 && !lexer->eof(lexer)) {
        lexer->mark_end(lexer);
        lexer->result_symbol = IMPORT_NEWLINE;
        return true;
      }
    }
    if (valid_symbols[NEWLINE]) {
      lexer->result_symbol = NEWLINE;
      return true;
    }
    return false;
  }

  return false;
}
