# Gin for VS Code and VSIX

This folder contains a standard VS Code extension: syntax highlighting for `.gin` files and a client for the [`ginlsp`](../../tools/ginlsp) language server (stdio).

## Prerequisites

Build or install `ginlsp` and ensure it is on your `PATH`, or set **Gin: Ginlsp Path** (`gin.ginlsp.path`) in settings to the full path of the binary.

Example from the repository root:

```bash
cargo build --release -p ginlsp
# Binary at target/release/ginlsp
```

## Local development

```bash
cd editors/vscode
npm ci
npm run compile
```

Press **F5** in VS Code with this folder opened to run an Extension Development Host.

## Package a VSIX

```bash
npm run package
```

This runs `vsce package` and writes `gin-<version>.vsix` in this directory.

## Publish to Open VSX

Open VSX uses the same VSIX format as the Microsoft Marketplace. VS Code forks install extensions from [Open VSX](https://open-vsx.org/).

1. Create an account on Open VSX and [claim a namespace](https://github.com/eclipse/openvsx/wiki/Publishing-Extensions) matching your `publisher` field in `package.json` (currently `patrickett`).
2. Create a [personal access token](https://open-vsx.org/user-settings/tokens).
3. From `editors/vscode` after `npm run package`, `vsce` writes `gin-<version>.vsix` (see the `name` field in `package.json`). Publish that file:

   ```bash
   npx ovsx publish gin-0.1.0.vsix -p <YOUR_TOKEN>
   ```

   Or publish from the extension directory (uses `package.json` metadata):

   ```bash
   npx ovsx publish -p <YOUR_TOKEN>
   ```

   Run `npx ovsx publish --help` for flags supported by your `ovsx` version.

For CI, store the token as a repository secret (for example `OVSX_PAT`) and add a guarded workflow step that runs `ovsx publish` only when you intend to release (for example on a dedicated tag or manual `workflow_dispatch`). The repository workflow [`.github/workflows/vscode-extension.yml`](../../.github/workflows/vscode-extension.yml) currently **builds** the VSIX and uploads it as an artifact; publishing remains an explicit local or follow-up automation step.

## Publish to the Microsoft Visual Studio Marketplace

Requires a [Azure DevOps publisher](https://code.visualstudio.com/api/working-with-extensions/publishing-extension) and `vsce login`. Then:

```bash
npm run publish:marketplace
```

## Versioning

Bump `"version"` in `package.json` before each publish so registries accept the new artifact.
