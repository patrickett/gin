const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { before, after, test } = require('node:test');
const { Query } = require('web-tree-sitter');
const { loadGrammar } = require('./grammar.cjs');
const { semanticTokens, TOKEN_TYPES, TOKEN_MODIFIERS } = require('../out/semanticTokens');
const cases = require('../../tree-sitter-gin/test/conformance.json');
let grammar;
before(async () => { grammar = await loadGrammar(); });
after(() => grammar?.dispose());

function highlight(source) {
  const tree = grammar.parser.parse(source);
  try {
    const tokens = semanticTokens(grammar.query.captures(tree.rootNode), source.split(/\r?\n/));
    return { tree: tree.rootNode.toString(), hasError: tree.rootNode.hasError, tokens };
  } finally {
    tree.delete();
  }
}

for (const entry of cases.filter(c => c.valid !== undefined)) {
  test(`shared grammar: ${entry.name}`, () => {
    const result = highlight(entry.source);
    assert.equal(result.hasError, !entry.valid, result.tree);
  });
}

test('all shared queries compile and the packaged query matches the source', () => {
  const queryDir = path.resolve(__dirname, '../../tree-sitter-gin/queries');
  for (const file of fs.readdirSync(queryDir).filter(f => f.endsWith('.scm'))) {
    const query = new Query(grammar.language, fs.readFileSync(path.join(queryDir, file), 'utf8'));
    query.delete();
  }
  assert.deepEqual(fs.readFileSync(path.join(queryDir, 'highlights.scm')),
    fs.readFileSync(path.resolve(__dirname, '../queries/highlights.scm')));
});

test('semantic tokens are sorted, non-overlapping, and inside UTF-16 line bounds', () => {
  const source = String.raw`msg := "🫖 hello \(outer(inner(1))) done"` + '\r\nnext := 2';
  const { tokens } = highlight(source);
  const lines = source.split(/\r?\n/);
  let previous;
  for (const token of tokens) {
    assert.ok(token.length > 0);
    assert.ok(token.start + token.length <= lines[token.row].length);
    if (previous) assert.ok(token.row > previous.row || token.start >= previous.start + previous.length);
    previous = token;
  }
  const inner = tokens.find(t => t.row === 0 && t.start === source.indexOf('inner'));
  assert.equal(TOKEN_TYPES[inner.type], 'function');
  assert.equal(inner.modifiers, 0);
  assert.ok(tokens.some(t => t.row === 1 && TOKEN_TYPES[t.type] === 'number'));
});

test('bindings, functions, attributes and compiler queries keep distinct scopes', () => {
  const source = '#inline\nf(value own T) T: value\nconstant := f(x)\nsize := #size(T)';
  const { tokens, hasError, tree } = highlight(source);
  assert.equal(hasError, false, tree);
  function token(row, text) {
    const start = source.split('\n')[row].indexOf(text);
    return tokens.find(t => t.row === row && t.start === start);
  }
  assert.equal(TOKEN_TYPES[token(0, 'inline').type], 'macro');
  assert.equal(TOKEN_TYPES[token(1, 'f').type], 'function');
  assert.equal(TOKEN_TYPES[token(1, 'value').type], 'parameter');
  assert.equal(TOKEN_TYPES[token(1, 'own').type], 'keyword');
  assert.equal(TOKEN_TYPES[token(2, 'constant').type], 'variable');
  const query = tokens.find(t => t.row === 3 && t.start === source.split('\n')[3].lastIndexOf('size'));
  assert.equal(TOKEN_TYPES[query.type], 'function');
  assert.equal(query.modifiers, 1 << TOKEN_MODIFIERS.indexOf('defaultLibrary'));
});

test('unterminated format strings do not color the next line as string content', () => {
  const source = 'msg := "unfinished\nnext := 42';
  const { tokens } = highlight(source);
  assert.ok(tokens.some(t => t.row === 1 && TOKEN_TYPES[t.type] === 'number'));
});

test('operator grouping follows the handwritten Pratt parser', () => {
  const tree = grammar.parser.parse('logical := a or b and c\nrange := a to b to c\naddress := @a + b\nnegative := -a + b');
  try {
    assert.equal(tree.rootNode.hasError, false, tree.rootNode.toString());
    const values = tree.rootNode.namedChildren.map(n => n.childForFieldName('value'));
    assert.equal(values[0].childForFieldName('operator').text, 'or');
    assert.equal(values[0].childForFieldName('right').childForFieldName('operator').text, 'and');
    assert.equal(values[1].childForFieldName('start').type, 'range_expression');
    assert.equal(values[2].type, 'unary_expression');
    assert.equal(values[2].childForFieldName('operand').type, 'binary_expression');
    assert.equal(values[3].type, 'binary_expression');
    assert.equal(values[3].childForFieldName('left').type, 'unary_expression');
  } finally {
    tree.delete();
  }
});

test('shared query covers every reserved lexer keyword', () => {
  const lexer = fs.readFileSync(path.resolve(__dirname, '../../../crates/lexer/src/token.rs'), 'utf8');
  const query = fs.readFileSync(path.resolve(__dirname, '../../tree-sitter-gin/queries/highlights.scm'), 'utf8');
  const keywords = lexer.match(/define_tokens!\s*\{\s*keywords\s*\{([\s\S]*?)\}/)?.[1] ?? '';
  const words = [...keywords.matchAll(/[A-Za-z]+ => "([a-z]+)",/g)].map(m => m[1]);
  assert.ok(words.length > 0);
  for (const word of words) assert.ok(query.includes(JSON.stringify(word)), word);
});

test('consecutive imports retain highlighting after editing selected exports', () => {
  let source = "use '../primitive' Byte, SignedBigInt\nuse '../string' String\nuse core.io.platform as platform\n";
  let tree = grammar.parser.parse(source);
  function assertImports() {
    assert.equal(tree.rootNode.hasError, false, tree.rootNode.toString());
    assert.deepEqual(tree.rootNode.namedChildren.map(node => node.type),
      ['use_statement', 'use_statement', 'use_statement']);
    const captures = grammar.query.captures(tree.rootNode);
    for (const row of [0, 1, 2]) {
      assert.ok(captures.some(c => c.name === 'keyword' && c.node.text === 'use'
        && c.node.startPosition.row === row));
    }
    assert.ok(captures.some(c => c.name === 'string' && c.node.text === "'../string'"));
    assert.ok(captures.some(c => c.name === 'type' && c.node.text === 'String'));
    assert.ok(captures.some(c => c.name === 'keyword' && c.node.text === 'as'));
  }
  try {
    assertImports();
    const start = source.indexOf('Byte');
    const replacement = 'Byte as Octet';
    tree.edit({
      startIndex: start,
      oldEndIndex: start + 4,
      newEndIndex: start + replacement.length,
      startPosition: { row: 0, column: start },
      oldEndPosition: { row: 0, column: start + 4 },
      newEndPosition: { row: 0, column: start + replacement.length },
    });
    source = source.slice(0, start) + replacement + source.slice(start + 4);
    const edited = grammar.parser.parse(source, tree);
    tree.delete();
    tree = edited;
    assertImports();
  } finally {
    tree.delete();
  }
});
