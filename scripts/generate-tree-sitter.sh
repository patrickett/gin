#!/usr/bin/env bash
# Regenerate tree-sitter C artifacts and WASM for Zed + VS Code from editors/tree-sitter-gin.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GRAMMAR_DIR="${ROOT}/editors/tree-sitter-gin"
ZED_WASM="${ROOT}/editors/zed/grammars/gin.wasm"
VSCODE_WASM_DIR="${ROOT}/editors/vscode/wasm"
VSCODE_GIN_WASM="${VSCODE_WASM_DIR}/gin.wasm"
VSCODE_QUERIES="${ROOT}/editors/vscode/queries"

cd "${GRAMMAR_DIR}"

TREE_SITTER="${GRAMMAR_DIR}/node_modules/.bin/tree-sitter"
if [[ ! -x "${TREE_SITTER}" ]]; then
  echo "error: install grammar dependencies with npm install --prefix editors/tree-sitter-gin" >&2
  exit 1
fi

CLI_VERSION="$(node -p 'require("./package.json").devDependencies["tree-sitter-cli"]')"
if [[ "$("${TREE_SITTER}" --version)" != "tree-sitter ${CLI_VERSION}"* ]]; then
  echo "error: reinstall grammar dependencies to use tree-sitter ${CLI_VERSION}" >&2
  exit 1
fi

"${TREE_SITTER}" generate --abi 14
"${TREE_SITTER}" build --wasm

WASM_SRC="${GRAMMAR_DIR}/tree-sitter-gin.wasm"
if [[ ! -f "${WASM_SRC}" ]]; then
  WASM_SRC="${GRAMMAR_DIR}/gin.wasm"
fi
if [[ ! -f "${WASM_SRC}" ]]; then
  echo "error: WASM output not found after build" >&2
  exit 1
fi

mkdir -p "$(dirname "${ZED_WASM}")" "${VSCODE_WASM_DIR}" "${VSCODE_QUERIES}"
cp "${WASM_SRC}" "${ZED_WASM}"
cp "${WASM_SRC}" "${VSCODE_GIN_WASM}"
cp "${GRAMMAR_DIR}/queries/highlights.scm" "${VSCODE_QUERIES}/highlights.scm"

echo "==> wrote ${ZED_WASM}"
echo "==> wrote ${VSCODE_GIN_WASM}"
echo "==> wrote ${VSCODE_QUERIES}/highlights.scm"
