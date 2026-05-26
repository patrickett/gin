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

if ! command -v npx >/dev/null 2>&1; then
  echo "error: npx is required (install Node.js)" >&2
  exit 1
fi

echo "==> tree-sitter generate (ABI 14)"
npx --yes tree-sitter-cli@0.22.6 generate --abi 14

echo "==> tree-sitter build --wasm"
npx --yes tree-sitter-cli@0.22.6 build --wasm

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
