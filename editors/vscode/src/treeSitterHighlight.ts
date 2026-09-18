import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import { Language, Parser, Query } from "web-tree-sitter";
import { semanticTokens, TOKEN_MODIFIERS, TOKEN_TYPES } from "./semanticTokens";

const LEGEND = new vscode.SemanticTokensLegend([...TOKEN_TYPES], [...TOKEN_MODIFIERS]);
let initPromise: Promise<void> | undefined;

export async function registerGinTreeSitter(context: vscode.ExtensionContext): Promise<void> {
  let language: Language;
  let query: Query;
  try {
    const wasmDir = context.asAbsolutePath("wasm");
    initPromise ??= Parser.init({ locateFile: (script: string) => path.join(wasmDir, script) });
    await initPromise;
    language = await Language.load(fs.readFileSync(path.join(wasmDir, "gin.wasm")));
    query = new Query(language, fs.readFileSync(context.asAbsolutePath("queries/highlights.scm"), "utf8"));
  } catch (err: unknown) {
    initPromise = undefined;
    const message = err instanceof Error ? err.message : String(err);
    void vscode.window.showWarningMessage(`Gin: tree-sitter highlighting unavailable (${message}).`);
    return;
  }

  const parser = new Parser();
  parser.setLanguage(language);
  context.subscriptions.push({ dispose() { parser.delete(); query.delete(); } });
  context.subscriptions.push(vscode.languages.registerDocumentSemanticTokensProvider(
    [{ language: "gin", scheme: "file" }, { language: "gin", scheme: "untitled" }],
    {
      provideDocumentSemanticTokens(document) {
        const source = document.getText();
        const tree = parser.parse(source);
        const builder = new vscode.SemanticTokensBuilder(LEGEND);
        try {
          if (tree) {
            for (const token of semanticTokens(query.captures(tree.rootNode), source.split(/\r?\n/))) {
              builder.push(token.row, token.start, token.length, token.type, token.modifiers);
            }
          }
          return builder.build();
        } finally {
          tree?.delete();
        }
      },
    },
    LEGEND,
  ));
}
