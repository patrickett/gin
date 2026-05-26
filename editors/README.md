## Supported Editors

- **[VS Code / VSIX](./vscode/)** — tree-sitter semantic highlighting and `ginlsp` via `vscode-languageclient`
- **[Zed](./zed/)** — tree-sitter grammar (WASM) and `ginlsp`

## Grammar source of truth

All editor highlighting uses one grammar:

- **Source:** [`tree-sitter-gin/`](tree-sitter-gin/) (`grammar.js`, `queries/*.scm`)
- **Regenerate** after grammar or query changes (from repo root):

```bash
./scripts/generate-tree-sitter.sh
```

This writes **local build artifacts** (not committed to git):

- `editors/tree-sitter-gin/src/parser.c`, `grammar.json`, `node-types.json`
- `editors/zed/grammars/gin.wasm`
- `editors/vscode/wasm/gin.wasm` and `editors/vscode/queries/highlights.scm`

Run this after clone and before building the `tree-sitter-gin` crate, Zed, or the VS Code extension.

The compiler (`crates/parser`) remains the authority for diagnostics and navigation via `ginlsp`.

## VS Code / VSIX setup

1. Build `ginlsp` and ensure it is on your `PATH`, or set **Gin: Ginlsp Path** (`gin.ginlsp.path`).

```bash
cargo build --release -p ginlsp
```

2. Regenerate tree-sitter WASM if needed:

```bash
./scripts/generate-tree-sitter.sh
cd editors/vscode && npm ci && npm run compile
```

3. Install the extension: open `editors/vscode` in VS Code and press **F5**, or run `npm run package` and install the `.vsix`.

## Zed setup

1. Regenerate WASM: `./scripts/generate-tree-sitter.sh`
2. Build `ginlsp` binaries (optional, for bundled LSP): `cd editors/zed && ./build-binaries.sh`
3. Install the `editors/zed` folder as a dev extension in Zed.

Zed query files under `zed/languages/gin/*.scm` symlink into `tree-sitter-gin/queries/`.
