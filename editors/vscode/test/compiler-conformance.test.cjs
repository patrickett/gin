const assert = require('node:assert/strict');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { before, after, test } = require('node:test');
const { loadGrammar } = require('./grammar.cjs');
const cases = require('./fixtures/parser-conformance.json');
const root = path.resolve(__dirname, '../../..');
let grammar;
let results;

before(async () => {
  grammar = await loadGrammar();
  const run = spawnSync('cargo', [
    'run', '--quiet', '--locked', '--manifest-path',
    path.join(root, 'editors/tree-sitter-gin/test/compiler/Cargo.toml'),
  ], {
    input: JSON.stringify(cases), encoding: 'utf8', timeout: 300_000,
    maxBuffer: 8 * 1024 * 1024,
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
  Comment: /^comment(\.|$)/,
  DocComment: /^comment(\.|$)/,
  ModuleDocComment: /^comment(\.|$)/,
  Pound: /^keyword\.modifier$/,
};
const punctuation = new Set([
  'ParenOpen', 'ParenClose', 'BracketOpen', 'BracketClose', 'CurlyOpen',
  'CurlyClose', 'Comma', 'ColonSemi', 'Dot',
]);
const operators = new Set([
  'At', 'ArrowLeft', 'Colon', 'ColonEq', 'ColonColon', 'Plus', 'Minus',
  'Star', 'Slash', 'SlashOr', 'Percent', 'Ampersand', 'Pipe', 'Caret',
  'ShiftLeft', 'ShiftRight', 'Eq', 'EqEq', 'NotEq', 'Less', 'LessEq',
  'Greater', 'GreaterEq', 'Infer', 'Question',
]);
const lexicalKinds = new Set([
  'Extern', 'Continue', 'Private', 'Return', 'Break', 'Loop', 'Ref', 'Mut',
  'Own', 'Deref', 'Domain', 'Then', 'Do', 'To', 'Thus', 'When', 'Where',
  'Else', 'SelfInstance', 'For', 'While', 'Use', 'Has', 'And', 'As', 'If',
  'In', 'Is', 'Of', 'Not', 'Or', 'SelfTag', 'Ampersand', 'ArrowLeft', 'At',
  'BracketClose', 'BracketOpen', 'Caret', 'Colon', 'ColonColon', 'ColonEq',
  'ColonSemi', 'Comma', 'Comment', 'CurlyClose', 'CurlyOpen', 'Dedent',
  'DocComment', 'ModuleDocComment', 'Dot', 'Eq', 'EqEq', 'Float',
  'FormatStringDelim', 'Greater', 'GreaterEq', 'Id', 'Indent', 'Infer', 'Int',
  'Less', 'LessEq', 'MalformedInt', 'MalformedFloat', 'Minus', 'Newline',
  'NotEq', 'ParenClose', 'ParenOpen', 'Percent', 'Pipe', 'Plus', 'Pound',
  'Question', 'ShiftLeft', 'ShiftRight', 'Slash', 'SlashOr', 'Star', 'String',
  'Tag', 'UnterminatedString', 'Unknown',
]);

for (const [index, entry] of cases.entries()) {
  test(`source parser and editor agree: ${entry.name}`, () => {
    const { tokens, diagnostics } = results[index];
    assert.equal(diagnostics.length === 0, entry.accepted, diagnostics.join('; '));
    for (const token of tokens) assert.ok(lexicalKinds.has(token.kind), token.kind);
    const tree = grammar.parser.parse(entry.source);
    try {
      if (entry.accepted) {
        assert.equal(tree.rootNode.hasError, false, tree.rootNode.toString());
        const captures = grammar.query.captures(tree.rootNode);
        for (const token of tokens) {
          if (token.kind === 'Newline') continue;
          const queryName = token.kind === 'Id'
            && entry.source.slice(0, token.start).trimEnd().endsWith('#');
          const expected = queryName ? /^keyword\.modifier$/ : scopes[token.kind]
            ?? (punctuation.has(token.kind) ? /^punctuation\./
              : operators.has(token.kind) ? /^operator$/ : /^keyword(\.|$)/);
          const capture = captures.find(item =>
            item.node.startIndex <= token.start && item.node.endIndex >= token.end);
          assert.ok(capture, `${token.kind}: ${entry.source.slice(token.start, token.end)}`);
          assert.match(capture.name, expected,
            `${token.kind} ${entry.source.slice(token.start, token.end)}: ${capture.name}`);
        }
      }
    } finally {
      tree.delete();
    }
  });
}
