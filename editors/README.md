## Supported Editors

- **[Zed](./zed/)** - Tree-sitter grammar and language server support

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
