import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import { Language, Parser, Query } from "web-tree-sitter";

const TOKEN_TYPES = [
  "namespace",
  "type",
  "class",
  "enum",
  "interface",
  "struct",
  "typeParameter",
  "parameter",
  "variable",
  "property",
  "enumMember",
  "function",
  "method",
  "macro",
  "keyword",
  "modifier",
  "comment",
  "string",
  "number",
  "regexp",
  "operator",
] as const;

const TOKEN_MODIFIERS = [
  "declaration",
  "definition",
  "readonly",
  "static",
  "deprecated",
  "abstract",
  "async",
  "modification",
  "documentation",
  "defaultLibrary",
] as const;

const LEGEND = new vscode.SemanticTokensLegend(
  [...TOKEN_TYPES],
  [...TOKEN_MODIFIERS],
);

const TYPE_INDEX: Record<string, number> = Object.fromEntries(
  TOKEN_TYPES.map((t, i) => [t, i]),
);

const MOD_INDEX: Record<string, number> = Object.fromEntries(
  TOKEN_MODIFIERS.map((m, i) => [m, 1 << i]),
);

let initPromise: Promise<{
  language: Language;
  query: Query;
}> | undefined;

function mod(...names: string[]): number {
  return names.reduce((acc, n) => acc | (MOD_INDEX[n] ?? 0), 0);
}

/** Map tree-sitter highlight capture names to VS Code semantic token indices. */
function mapCapture(captureName: string): { type: number; modifiers: number } | null {
  const name = captureName.startsWith("@") ? captureName.slice(1) : captureName;

  if (name === "variable.builtin") {
    return { type: TYPE_INDEX.variable, modifiers: mod("defaultLibrary") };
  }
  if (name === "variable.parameter") {
    return { type: TYPE_INDEX.parameter, modifiers: 0 };
  }
  if (name === "function.call") {
    return { type: TYPE_INDEX.function, modifiers: mod("defaultLibrary") };
  }
  if (name === "function.method") {
    return { type: TYPE_INDEX.method, modifiers: 0 };
  }
  if (name === "constructor") {
    return { type: TYPE_INDEX.type, modifiers: 0 };
  }
  if (name.startsWith("keyword")) {
    return { type: TYPE_INDEX.keyword, modifiers: 0 };
  }
  if (name.startsWith("punctuation")) {
    return null;
  }
  if (name === "string.escape" || name === "string.special") {
    return { type: TYPE_INDEX.string, modifiers: 0 };
  }

  const base = name.split(".")[0];
  const typeKey = base as (typeof TOKEN_TYPES)[number];
  if (TYPE_INDEX[typeKey] === undefined) {
    return null;
  }
  return { type: TYPE_INDEX[typeKey], modifiers: 0 };
}

async function loadGrammar(context: vscode.ExtensionContext): Promise<{
  language: Language;
  query: Query;
}> {
  const wasmDir = context.asAbsolutePath("wasm");
  await Parser.init({
    locateFile(scriptName: string) {
      return path.join(wasmDir, scriptName);
    },
  });

  const ginWasmPath = path.join(wasmDir, "gin.wasm");
  const ginWasm = fs.readFileSync(ginWasmPath);
  const language = await Language.load(ginWasm);

  const bundledQuery = context.asAbsolutePath(
    path.join("queries", "highlights.scm"),
  );
  const monorepoQuery = path.join(
    context.extensionPath,
    "..",
    "tree-sitter-gin",
    "queries",
    "highlights.scm",
  );
  const highlightsPath = fs.existsSync(bundledQuery)
    ? bundledQuery
    : monorepoQuery;
  const querySource = fs.readFileSync(highlightsPath, "utf8");
  const query = new Query(language, querySource);

  return { language, query };
}

function pushNodeTokens(
  builder: vscode.SemanticTokensBuilder,
  document: vscode.TextDocument,
  node: { startPosition: { row: number; column: number }; endPosition: { row: number; column: number } },
  tokenType: number,
  tokenModifiers: number,
): void {
  const start = node.startPosition;
  const end = node.endPosition;
  for (let row = start.row; row <= end.row; row++) {
    const line = document.lineAt(row).text;
    const startChar = row === start.row ? start.column : 0;
    const endChar = row === end.row ? end.column : line.length;
    const length = endChar - startChar;
    if (length > 0) {
      builder.push(row, startChar, length, tokenType, tokenModifiers);
    }
  }
}

async function ensureInitialized(
  context: vscode.ExtensionContext,
): Promise<{ language: Language; query: Query }> {
  if (!initPromise) {
    initPromise = loadGrammar(context);
  }
  return initPromise;
}

export async function registerGinTreeSitter(
  context: vscode.ExtensionContext,
): Promise<void> {
  let language: Language;
  let query: Query;
  try {
    ({ language, query } = await ensureInitialized(context));
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    void vscode.window.showWarningMessage(
      `Gin: tree-sitter highlighting unavailable (${message}).`,
    );
    return;
  }

  const selector: vscode.DocumentSelector = { language: "gin", scheme: "file" };

  const provider: vscode.DocumentSemanticTokensProvider = {
    provideDocumentSemanticTokens(document) {
      const parser = new Parser();
      parser.setLanguage(language);
      const tree = parser.parse(document.getText());
      if (!tree) {
        return new vscode.SemanticTokensBuilder(LEGEND).build();
      }
      const builder = new vscode.SemanticTokensBuilder(LEGEND);

      for (const match of query.matches(tree.rootNode)) {
        for (const capture of match.captures) {
          const mapped = mapCapture(capture.name);
          if (!mapped) {
            continue;
          }
          pushNodeTokens(builder, document, capture.node, mapped.type, mapped.modifiers);
        }
      }

      return builder.build();
    },
  };

  context.subscriptions.push(
    vscode.languages.registerDocumentSemanticTokensProvider(
      selector,
      provider,
      LEGEND,
    ),
  );
}
