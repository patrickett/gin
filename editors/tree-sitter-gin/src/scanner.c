#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <tree_sitter/parser.h>

enum TokenType {
  NEWLINE, UNTERMINATED_STRING, ERROR_RECOVERY, IMPORT_NEWLINE, INDENT, DEDENT,
  PRIVATE_INDENT,
  RETURN_END, WHEN_ARM_END, EMPTY_HAS_END,
};

enum { INDENT_DEPTH_MAX = 128 };

typedef struct {
  uint16_t indents[INDENT_DEPTH_MAX];
  uint16_t depth;
} Scanner;

void *tree_sitter_gin_external_scanner_create() {
  Scanner *scanner = calloc(1, sizeof(Scanner));
  if (scanner != NULL)
    scanner->depth = 1;
  return scanner;
}

void tree_sitter_gin_external_scanner_destroy(void *payload) {
  free(payload);
}

unsigned tree_sitter_gin_external_scanner_serialize(void *payload,
                                                  char *buffer) {
  const Scanner *scanner = payload;
  const unsigned size = sizeof(scanner->depth) +
                        scanner->depth * sizeof(scanner->indents[0]);
  memcpy(buffer, &scanner->depth, sizeof(scanner->depth));
  memcpy(buffer + sizeof(scanner->depth), scanner->indents,
         scanner->depth * sizeof(scanner->indents[0]));
  return size;
}

void tree_sitter_gin_external_scanner_deserialize(void *payload,
                                                const char *buffer,
                                                unsigned length) {
  Scanner *scanner = payload;
  memset(scanner, 0, sizeof(*scanner));
  scanner->depth = 1;
  if (length < sizeof(scanner->depth))
    return;
  uint16_t depth;
  memcpy(&depth, buffer, sizeof(depth));
  if (depth == 0 || depth > INDENT_DEPTH_MAX ||
      length != sizeof(depth) + depth * sizeof(scanner->indents[0]))
    return;
  scanner->depth = depth;
  memcpy(scanner->indents, buffer + sizeof(depth),
         depth * sizeof(scanner->indents[0]));
}

bool tree_sitter_gin_external_scanner_scan(void *payload, TSLexer *lexer,
                                         const bool *valid_symbols) {
  Scanner *scanner = payload;
  if (scanner == NULL || valid_symbols[ERROR_RECOVERY])
    return false;

  if (valid_symbols[UNTERMINATED_STRING] &&
      (lexer->lookahead == '\n' || lexer->eof(lexer))) {
    lexer->result_symbol = UNTERMINATED_STRING;
    return true;
  }

  unsigned column = lexer->get_column(lexer);
  bool newline = false;
  lexer->mark_end(lexer);
  while (lexer->lookahead == ' ' || lexer->lookahead == '\t' ||
         lexer->lookahead == '\r' || lexer->lookahead == '\n') {
    if (lexer->lookahead == '\n') {
      newline = true;
      column = 0;
    } else if (lexer->lookahead == '\t') {
      column += 4;
    } else if (lexer->lookahead == ' ') {
      column++;
    }
    lexer->advance(lexer, true);
  }

  const unsigned current = scanner->indents[scanner->depth - 1];
  if (valid_symbols[RETURN_END] && (newline || lexer->eof(lexer))) {
    lexer->result_symbol = RETURN_END;
    return true;
  }
  if (valid_symbols[EMPTY_HAS_END] && (newline || lexer->eof(lexer)) &&
      column <= current) {
    lexer->result_symbol = EMPTY_HAS_END;
    return true;
  }
  if (valid_symbols[WHEN_ARM_END] && (newline || lexer->eof(lexer)) &&
      column <= current) {
    lexer->result_symbol = WHEN_ARM_END;
    return true;
  }
  if (lexer->eof(lexer) || (newline && column < current)) {
    if (scanner->depth > 1) {
      if (!valid_symbols[DEDENT])
        return false;
      scanner->depth--;
      lexer->result_symbol = DEDENT;
      return true;
    }
  }

  if (newline && (valid_symbols[INDENT] || valid_symbols[PRIVATE_INDENT]) &&
      column > current &&
      scanner->depth < INDENT_DEPTH_MAX && column <= UINT16_MAX &&
      !lexer->eof(lexer)) {
    scanner->indents[scanner->depth++] = column;
    lexer->mark_end(lexer);
    lexer->result_symbol = valid_symbols[PRIVATE_INDENT] ? PRIVATE_INDENT : INDENT;
    return true;
  }

  if (!newline)
    return false;
  lexer->mark_end(lexer);
  if (valid_symbols[IMPORT_NEWLINE] && column > 0 && !lexer->eof(lexer)) {
    lexer->result_symbol = IMPORT_NEWLINE;
    return true;
  }
  if (valid_symbols[NEWLINE]) {
    lexer->result_symbol = NEWLINE;
    return true;
  }
  return false;
}
