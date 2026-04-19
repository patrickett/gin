## Supported Editors

- **[VS Code / VSIX](./vscode/)** - TextMate highlighting, `ginlsp` via `vscode-languageclient`, VSIX packaging and Open VSX–compatible publishing
- **[Zed](./zed/)** - Tree-sitter grammar and language server support

## VS Code / VSIX setup

1. Build `ginlsp` and ensure it is on your `PATH`, or configure **Gin: Ginlsp Path** (`gin.ginlsp.path`).
  ```bash
   cargo build --release -p ginlsp
  ```
2. Install the extension:
  - **From source**: open `editors/vscode` in VS Code, run **Run and Debug** (F5), or run `npm ci && npm run package` and install the generated `.vsix` via **Extensions: Install from VSIX…**
  - **From Open VSX**: after the extension is published, search for **Gin** (publisher `patrickett`) in the marketplace UI your editor uses, or follow the publish steps in `[editors/vscode/README.md](./vscode/README.md)`.
3. Open a `.gin` file to activate the language server.

## Zed Setup

1. Build the language server binaries:
  ```bash
   cd editors/zed
   ./build-binaries.sh
  ```
2. Install the extension as a dev extension in Zed:
  - Open Zed
  - Go to the extensions page (`zed: extensions`)
  - Click "Install Dev Extension"
  - Select the `editors/zed` directory
3. Open a `.gin` file to activate the extension.

