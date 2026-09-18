const fs = require('node:fs');
const path = require('node:path');
const { Language, Parser, Query } = require('web-tree-sitter');

async function loadGrammar() {
  const wasm = path.resolve(__dirname, '../wasm');
  await Parser.init({ locateFile: file => path.join(wasm, file) });
  const language = await Language.load(path.join(wasm, 'gin.wasm'));
  const query = new Query(language, fs.readFileSync(path.resolve(__dirname, '../queries/highlights.scm'), 'utf8'));
  const parser = new Parser();
  parser.setLanguage(language);
  return { language, query, parser, dispose() { parser.delete(); query.delete(); } };
}
module.exports = { loadGrammar };
