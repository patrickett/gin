# Gin for VS Code

Tree-sitter syntax highlighting for `.gin` files and a client for `ginlsp` over stdio. Highlighting works independently of the language server, including in untitled Gin documents.

## Install a VSIX

In VS Code, run **Extensions: Install from VSIX…** and select `gin-0.1.1.vsix`, then reload the window. Or use:

```sh
code --install-extension gin-0.1.1.vsix
```

This package targets VS Code and compatible extension hosts. It does not establish support for installing VSIX packages into the Codex desktop app; the Codex IDE extension can be used alongside Gin in VS Code.

For diagnostics and other language-server features, install `ginlsp` on your `PATH` or set `gin.ginlsp.path` to its absolute path. From the repository root:

```sh
cargo build --release -p ginlsp
```

## Build and test

Install Node.js, Rust (for compiler conformance tests), and Emscripten (for the grammar WASM build). From the repository root:

```sh
npm install --prefix editors/tree-sitter-gin
npm ci --prefix editors/vscode
bash scripts/generate-tree-sitter.sh
npm --prefix editors/tree-sitter-gin test
npm --prefix editors/vscode test
npm --prefix editors/vscode run test:compiler
npm --prefix editors/vscode run package
```

Packaging regenerates the shared parser and WASM, copies the highlight query, compiles TypeScript, and runs the highlighting tests. It writes `editors/vscode/gin-<version>.vsix`.

Edit the grammar and queries in `editors/tree-sitter-gin`; Zed and VS Code use that same source. Compiler conformance cases live in `editors/tree-sitter-gin/test/conformance.json`. The comparison runs the current Rust lexer and parser and checks accepted/rejected examples and highlight spans. Tree-sitter remains an editor parser with error recovery; `ginlsp` supplies authoritative syntax and semantic diagnostics.

## Local development

After generating the WASM, run `npm run watch` in this directory. Press **F5** with this folder open to launch an Extension Development Host.

## Publish

See [editors/README.md](../README.md) for Open VSX and Marketplace notes. Bump the version before publishing.
