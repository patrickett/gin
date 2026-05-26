# Gin for VS Code and VSIX

This extension provides **tree-sitter semantic highlighting** for `.gin` files and a client for the [`ginlsp`](../../tools/ginlsp) language server (stdio).

## Prerequisites

Build or install `ginlsp` and ensure it is on your `PATH`, or set **Gin: Ginlsp Path** (`gin.ginlsp.path`) in settings.

From the repository root:

```bash
cargo build --release -p ginlsp
./scripts/generate-tree-sitter.sh
```

## Local development

```bash
cd editors/vscode
npm ci
npm run compile
```

Press **F5** with this folder open to run an Extension Development Host.

Highlighting uses `wasm/gin.wasm` and `queries/highlights.scm` (produced locally by `generate-tree-sitter.sh`; these paths are gitignored). In a dev checkout, queries can also load from `../tree-sitter-gin/queries/`. `postinstall` copies `tree-sitter.wasm` from `web-tree-sitter`.

## Package a VSIX

Ensure WASM is present (`../../scripts/generate-tree-sitter.sh`), then:

```bash
npm run package
```

This writes `gin-<version>.vsix` in this directory.

## Publish

See [editors/README.md](../README.md) for Open VSX and Marketplace notes.

## Versioning

Bump `"version"` in `package.json` before each publish.
