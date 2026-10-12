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

test('editing a bare return preserves indentation and the next declaration', () => {
  const source = 'run:\n\twhile ready do\n\t\tobserve\n\treturn\nnext: 1\n';
  const index = source.indexOf('return') + 'return'.length;
  const tree = grammar.parser.parse(source);
  try {
    assert.equal(tree.rootNode.hasError, false, tree.rootNode.toString());
    assert.equal(tree.rootNode.namedChildCount, 2);
    tree.edit({
      startIndex: index, oldEndIndex: index, newEndIndex: index + 2,
      startPosition: { row: 3, column: 7 },
      oldEndPosition: { row: 3, column: 7 },
      newEndPosition: { row: 3, column: 9 },
    });
    const updated = source.slice(0, index) + ' 0' + source.slice(index);
    const incremental = grammar.parser.parse(updated, tree);
    const fresh = grammar.parser.parse(updated);
    try {
      assert.equal(incremental.rootNode.hasError, false, incremental.rootNode.toString());
      assert.equal(incremental.rootNode.namedChildCount, 2);
      assert.equal(incremental.rootNode.toString(), fresh.rootNode.toString());
    } finally {
      incremental.delete();
      fresh.delete();
    }
  } finally {
    tree.delete();
  }
});

test('private blocks highlight their marker, grants, fields and methods', () => {
  const source = `Record has
    #private(client)
        secret Word
        hidden(self ref) Word: return self.secret
    visible Word
`;
  const { hasError, tree, tokens } = highlight(source);
  assert.equal(hasError, false, tree);
  assert.match(tree, /private_member_block/);
  const lines = source.split('\n');
  for (const [row, text, expected] of [
    [1, '#private', 'keyword'],
    [1, 'client', 'variable'],
    [2, 'secret', 'property'],
    [3, 'hidden', 'method'],
    [4, 'visible', 'property'],
  ]) {
    const token = tokens.find(token => token.row === row && token.start === lines[row].indexOf(text));
    assert.ok(token, text);
    assert.equal(TOKEN_TYPES[token.type], expected, text);
  }
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

test('bindings and functions retain their scopes alongside keyword attributes and queries', () => {
  const source = '#inline\nf(value own T) T: value\nconstant := f(x)\nsize := #size(T)';
  const { tokens, hasError, tree } = highlight(source);
  assert.equal(hasError, false, tree);
  function token(row, text) {
    const start = source.split('\n')[row].indexOf(text);
    return tokens.find(t => t.row === row && t.start === start);
  }
  assert.equal(TOKEN_TYPES[token(0, '#inline').type], 'keyword');
  assert.equal(TOKEN_TYPES[token(1, 'f').type], 'function');
  assert.equal(TOKEN_TYPES[token(1, 'value').type], 'parameter');
  assert.equal(TOKEN_TYPES[token(1, 'own').type], 'keyword');
  assert.equal(TOKEN_TYPES[token(2, 'constant').type], 'variable');
  const query = token(3, '#size');
  assert.equal(query.length, '#size'.length);
  assert.equal(TOKEN_TYPES[query.type], 'keyword');
  assert.equal(query.modifiers, 0);
});

test('consecutive private member attributes retain the same highlighting', () => {
  const source = `Session has
    output Output
    #private
    input SystemResult
    #private
    original Settings
    #private
    decoder Decoder
    #private
    active in 0 to 1
    #private(alloc)
    inspect(self ref): return self.input
`;
  const { tokens, hasError, tree } = highlight(source);
  assert.equal(hasError, false, tree);
  const lines = source.split('\n');
  for (const row of [2, 4, 6, 8, 10]) {
    const token = tokens.find(t => t.row === row && t.start === lines[row].indexOf('#'));
    assert.ok(token, `missing attribute on line ${row + 1}`);
    assert.equal(token.length, '#private'.length);
    assert.equal(TOKEN_TYPES[token.type], 'keyword');
    assert.equal(token.modifiers, 0);
  }
});

test('attributes and compiler queries share the has keyword scope', () => {
  const source = `Marker has
    #private
    value T
#inline
f(value T) T: return value
size := #size(T)
alignment := #alignment(T)
bits := #index_bits(T)
target := #target
arch := #target.arch
emitted := #emit(code, [value: value])
unfinished := #todo
`;
  const tree = grammar.parser.parse(source);
  try {
    assert.equal(tree.rootNode.hasError, false, tree.rootNode.toString());
    const captures = grammar.query.captures(tree.rootNode);
    const tokens = semanticTokens(captures, source.split('\n'));
    const hashes = [...source.matchAll(/#/g)];
    assert.equal(hashes.length, 9);
    const has = captures.find(c => c.node.text === 'has');
    assert.equal(has.name, 'keyword.modifier');
    for (const name of ['private', 'inline', 'size', 'alignment', 'index_bits', 'target', 'emit', 'todo']) {
      const matches = captures.filter(c => c.node.text === name && c.name === has.name);
      assert.ok(matches.length > 0, `${name} should share the has scope`);
      for (const capture of matches) {
        const { row, column } = capture.node.startPosition;
        const token = tokens.find(t => t.row === row && t.start <= column && t.start + t.length >= column + name.length);
        assert.ok(token, `missing token for ${name}`);
        assert.equal(TOKEN_TYPES[token.type], 'keyword');
        assert.equal(token.modifiers, 0);
      }
    }
    for (const match of hashes) {
      const prefix = source.slice(0, match.index).split('\n');
      const row = prefix.length - 1;
      const column = prefix.at(-1).length;
      const capture = captures.find(c => c.node.startIndex === match.index && c.node.text === '#');
      assert.ok(capture, `missing hash capture on line ${row + 1}`);
      assert.equal(capture.name, has.name);
      const token = tokens.find(t => t.row === row && t.start <= column && t.start + t.length > column);
      assert.ok(token, `missing hash token on line ${row + 1}`);
      assert.equal(TOKEN_TYPES[token.type], 'keyword');
      assert.equal(token.modifiers, 0);
    }
  } finally {
    tree.delete();
  }
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
  for (const word of words.filter(word => !["extern", "if", "loop", "break", "continue"].includes(word))) assert.ok(query.includes(JSON.stringify(word)), word);
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

const recordMethods = cases.find(c => c.name === 'record methods with range elements and destructuring');

test('record methods retain their declaration, bodies, and destructuring', () => {
  const tree = grammar.parser.parse(recordMethods.source);
  try {
    assert.equal(tree.rootNode.hasError, false, tree.rootNode.toString());
    const declarations = tree.rootNode.namedChildren;
    assert.deepEqual(declarations.map(n => n.type), ['declare_statement', 'declare_statement']);
    assert.deepEqual(declarations.map(n => n.childForFieldName('name').text), ['Packet', 'Printable']);
    const members = declarations[0].descendantsOfType('decl_member_list')[0];
    const methods = members.namedChildren.filter(n => n.type === 'record_signature');
    assert.deepEqual(methods.map(n => n.childForFieldName('name').text),
      ['from_values', 'copy', 'merge', 'append']);
    assert.equal(methods[0].descendantsOfType('sequence_type')[0].text, '[in Byte; count]');
    assert.deepEqual(methods[2].descendantsOfType('destructure_statement').map(n => n.text),
      ['Packet(values) := self', 'Packet(values: right) := other']);
    assert.equal(methods[2].descendantsOfType('attributes')[0].text, "#infix('+')");
  } finally {
    tree.delete();
  }
});

test('record method highlights resolve to the intended token categories', () => {
  const source = recordMethods.source;
  const { tokens, hasError, tree } = highlight(source);
  assert.equal(hasError, false, tree);
  for (const [context, word, type, modifier] of [
    ['Packet has', 'Packet', 'type'],
    ['has Copy and Release', 'has', 'keyword'],
    ['has Copy and Release', 'Copy', 'type'],
    ['has Copy and Release', 'and', 'keyword'],
    ['values Buffer(Byte)', 'values', 'property'],
    ['--- Copies the input elements.', '--- Copies the input elements.', 'comment', 'documentation'],
    ['from_values(source', 'from_values', 'method'],
    ['from_values(source', 'source', 'parameter'],
    ['[in Byte; count]', 'in', 'keyword'],
    ['[in Byte; count]', 'Byte', 'type'],
    ['[in Byte; count]', 'count', 'variable'],
    ['Buffer.with_capacity', 'with_capacity', 'method'],
    ['count as Length', 'as', 'keyword'],
    ['for index in 0 to count', 'for', 'keyword'],
    ['for index in 0 to count', 'index', 'variable'],
    ['for index in 0 to count', '0', 'number'],
    ['for index in 0 to count', 'to', 'keyword'],
    ['values.push', 'push', 'method'],
    ['source.[index]', 'source', 'variable'],
    ['source.[index]', 'index', 'variable'],
    ['return Self(values)', 'return', 'keyword'],
    ['return Self(values)', 'Self', 'type', 'defaultLibrary'],
    ['copy(self ref)', 'copy', 'method'],
    ['copy(self ref)', 'self', 'variable', 'defaultLibrary'],
    ['copy(self ref)', 'ref', 'keyword'],
    ['Self(values: self.values.copy)', 'values', 'property'],
    ["#infix('+')", '#infix', 'keyword'],
    ["#infix('+')", "'+'", 'string'],
    ['merge(self own', 'merge', 'method'],
    ['merge(self own', 'own', 'keyword'],
    ['other own Self', 'other', 'parameter'],
    ['Packet(values) := self', 'values', 'variable'],
    ['Packet(values) := self', ':=', 'operator'],
    ['Packet(values: right)', 'values', 'property'],
    ['Packet(values: right)', 'right', 'variable'],
    ['values.append(right)', 'append', 'method'],
    ['append(self mut', 'mut', 'keyword'],
    ['self.values.append(other.values)', 'append', 'method'],
    ['other.values', 'values', 'property'],
  ]) {
    const offset = source.indexOf(context);
    assert.notEqual(offset, -1, context);
    const start = offset + context.lastIndexOf(word);
    const prefix = source.slice(0, start).split('\n');
    const row = prefix.length - 1;
    const column = prefix.at(-1).length;
    const token = tokens.find(t => t.row === row && t.start === column && t.length === word.length);
    assert.ok(token, `${context}: missing token for ${word}`);
    assert.equal(TOKEN_TYPES[token.type], type, `${context}: ${word}`);
    assert.equal(token.modifiers, modifier ? 1 << TOKEN_MODIFIERS.indexOf(modifier) : 0, context);
  }
});
