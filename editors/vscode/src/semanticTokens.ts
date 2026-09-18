import type { QueryCapture } from "web-tree-sitter";

export const TOKEN_TYPES = [
  "namespace", "type", "class", "enum", "interface", "struct", "typeParameter",
  "parameter", "variable", "property", "enumMember", "function", "method",
  "macro", "keyword", "modifier", "comment", "string", "number", "regexp", "operator",
] as const;

export const TOKEN_MODIFIERS = [
  "declaration", "definition", "readonly", "static", "deprecated", "abstract",
  "async", "modification", "documentation", "defaultLibrary",
] as const;

export interface HighlightToken {
  row: number;
  start: number;
  length: number;
  type: number;
  modifiers: number;
}

function mapCapture(name: string): { type: number; modifiers: number } | null {
  const [base, detail] = name.split(".");
  const typeName = name === "variable.parameter" ? "parameter"
    : name === "function.method" ? "method"
    : base === "constructor" ? "type"
    : base === "attribute" ? "macro"
    : base;
  const type = TOKEN_TYPES.indexOf(typeName as typeof TOKEN_TYPES[number]);
  if (type === -1) return null;
  const modifier = detail === "builtin" ? "defaultLibrary"
    : detail === "documentation" ? "documentation" : undefined;
  return { type, modifiers: modifier ? 1 << TOKEN_MODIFIERS.indexOf(modifier) : 0 };
}

interface Candidate extends HighlightToken {
  end: number;
  extent: number;
  pattern: number;
}

export function semanticTokens(captures: QueryCapture[], lines: readonly string[]): HighlightToken[] {
  const rows = new Map<number, Candidate[]>();
  for (const capture of captures) {
    const mapped = mapCapture(capture.name);
    if (!mapped) continue;
    const { node } = capture;
    for (let row = node.startPosition.row; row <= node.endPosition.row; row++) {
      const start = row === node.startPosition.row ? node.startPosition.column : 0;
      const end = row === node.endPosition.row ? node.endPosition.column : lines[row].length;
      if (end <= start) continue;
      const candidates = rows.get(row) ?? [];
      candidates.push({
        row, start, end, length: end - start, ...mapped,
        extent: node.endIndex - node.startIndex, pattern: capture.patternIndex,
      });
      rows.set(row, candidates);
    }
  }

  const tokens: HighlightToken[] = [];
  for (const [row, candidates] of [...rows].sort(([a], [b]) => a - b)) {
    candidates.sort((a, b) => a.extent - b.extent || b.pattern - a.pattern);
    const boundaries = [...new Set(candidates.flatMap(c => [c.start, c.end]))].sort((a, b) => a - b);
    for (let i = 0; i < boundaries.length - 1; i++) {
      const start = boundaries[i];
      const end = boundaries[i + 1];
      const chosen = candidates.find(c => c.start <= start && c.end >= end);
      if (!chosen) continue;
      const previous = tokens.at(-1);
      if (previous?.row === row && previous.start + previous.length === start
        && previous.type === chosen.type && previous.modifiers === chosen.modifiers) {
        previous.length += end - start;
      } else {
        tokens.push({ row, start, length: end - start, type: chosen.type, modifiers: chosen.modifiers });
      }
    }
  }
  return tokens;
}
