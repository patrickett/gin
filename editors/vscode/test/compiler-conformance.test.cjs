const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { before, after, test } = require('node:test');
const { loadGrammar } = require('./grammar.cjs');
const cases = require('../../tree-sitter-gin/test/conformance.json').filter(c => c.valid !== undefined && c.compiler !== false);
const root = path.resolve(__dirname, '../../..');
const moduleRoot = path.join(root, 'modules');
const modules = fs.readdirSync(moduleRoot, { recursive: true })
  .filter(file => file.endsWith('.gin'))
  .sort()
  .map(file => ({ name: file, source: fs.readFileSync(path.join(moduleRoot, file), 'utf8'), valid: true }));
let grammar;
let results;
before(async () => {
  grammar = await loadGrammar();
  const run = spawnSync('cargo', [
    'run', '--quiet', '--locked', '--manifest-path',
    path.join(root, 'editors/tree-sitter-gin/test/compiler/Cargo.toml'),
  ], {
    input: JSON.stringify([...cases, ...modules]), encoding: 'utf8', timeout: 300_000,
    maxBuffer: 32 * 1024 * 1024,
    env: { ...process.env, CARGO_TARGET_DIR: path.join(root, 'target/vscode-grammar') },
  });
  assert.equal(run.status, 0, `${run.error ?? ''}\n${run.stderr}`);
  results = JSON.parse(run.stdout);
});
after(() => grammar?.dispose());

const scopes = {
  Id: /^(variable|function|property|type|attribute)(\.|$)/,
  Tag: /^(type|constructor)(\.|$)/,
  SelfInstance: /^variable\.builtin$/,
  SelfTag: /^type\.builtin$/,
  Int: /^number$/,
  Float: /^number$/,
  String: /^string$/,
  FormatStringDelim: /^string$/,
  FormatStringText: /^string(\.|$)/,
  FormatInterpStart: /^punctuation\.special$/,
  FormatInterpEnd: /^punctuation\.special$/,
  Comment: /^comment(\.|$)/,
  DocComment: /^comment(\.|$)/,
  ModuleDocComment: /^comment(\.|$)/,
  Pound: /^keyword\.modifier$/,
};
const punctuation = new Set(['ParenOpen', 'ParenClose', 'BracketOpen', 'BracketClose', 'CurlyOpen', 'CurlyClose', 'Comma', 'ColonSemi', 'Dot']);
const operators = new Set(['At', 'Colon', 'ColonEq', 'ColonColon', 'Plus', 'Minus', 'Star', 'Slash', 'Percent', 'Ampersand', 'Pipe', 'Caret', 'ShiftLeft', 'ShiftRight', 'Eq', 'EqEq', 'NotEq', 'Less', 'LessEq', 'Greater', 'GreaterEq', 'Infer', 'Question']);

for (const [i, entry] of cases.entries()) {
  test(`compiler agrees: ${entry.name}`, () => {
    const { tokens, diagnostics } = results[i];
    const tree = grammar.parser.parse(entry.source);
    try {
      const captures = grammar.query.captures(tree.rootNode).sort((a, b) =>
        (a.node.endIndex - a.node.startIndex) - (b.node.endIndex - b.node.startIndex)
        || b.patternIndex - a.patternIndex);
      if (entry.valid) {
        assert.deepEqual(diagnostics, [], entry.source);
        assert.equal(tree.rootNode.hasError, false, tree.rootNode.toString());
      } else {
        assert.ok(diagnostics.length > 0, entry.source);
        assert.equal(tree.rootNode.hasError, true, tree.rootNode.toString());
        return;
      }
      for (const token of tokens) {
        if (['Newline', 'Indent', 'Dedent'].includes(token.kind)) continue;
        const queryName = token.kind === 'Id' && entry.source.slice(0, token.start).trimEnd().endsWith('#');
        const expected = queryName ? /^keyword\.modifier$/ : scopes[token.kind] ?? (punctuation.has(token.kind)
          ? /^punctuation\./ : operators.has(token.kind) ? /^operator$/ : /^keyword(\.|$)/);
        const capture = captures.find(c => c.node.startIndex <= token.start && c.node.endIndex >= token.end);
        assert.ok(capture, `${token.kind}: ${entry.source.slice(token.start, token.end)}`);
        assert.match(capture.name, expected, `${token.kind} ${entry.source.slice(token.start, token.end)}: ${capture.name}`);
      }
    } finally {
      tree.delete();
    }
  });
}

for (const [i, entry] of modules.entries()) {
  test(`compiler-accepted module: ${entry.name}`, t => {
    const { diagnostics } = results[cases.length + i];
    if (diagnostics.length > 0) {
      t.skip(`compiler rejects this source: ${diagnostics.join('; ')}`);
      return;
    }
    const tree = grammar.parser.parse(entry.source);
    try {
      assert.equal(tree.rootNode.hasError, false, tree.rootNode.toString());
    } finally {
      tree.delete();
    }
  });
}
