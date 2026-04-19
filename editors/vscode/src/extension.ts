import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;
let lspOutput: vscode.OutputChannel | undefined;

function ginlspCommand(): string {
  const raw = vscode.workspace.getConfiguration("gin").get<string>("ginlsp.path");
  const trimmed = raw?.trim() ?? "";
  return trimmed.length > 0 ? trimmed : "ginlsp";
}

function createClient(outputChannel: vscode.OutputChannel): LanguageClient {
  const command = ginlspCommand();
  const serverOptions: ServerOptions = {
    run: { command, args: [], transport: TransportKind.stdio },
    debug: { command, args: [], transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "gin" }],
    outputChannel,
    outputChannelName: "Gin Language Server",
  };

  return new LanguageClient("ginlsp", "Gin Language Server", serverOptions, clientOptions);
}

export function activate(context: vscode.ExtensionContext): void {
  lspOutput = vscode.window.createOutputChannel("Gin Language Server");
  context.subscriptions.push(lspOutput);

  client = createClient(lspOutput);
  context.subscriptions.push(client);

  void client
    .start()
    .then(
      undefined,
      (err: unknown) => {
        const message = err instanceof Error ? err.message : String(err);
        void vscode.window.showErrorMessage(
          `Gin: could not start ginlsp (${ginlspCommand()}). ${message} Install ginlsp or set gin.ginlsp.path.`,
        );
      },
    );

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (!e.affectsConfiguration("gin.ginlsp.path")) {
        return;
      }
      void restartGinlspClient(context, { notifyOnSuccess: false });
    }),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("gin.restartLanguageServer", async () => {
      await restartGinlspClient(context, { notifyOnSuccess: true });
    }),
  );
}

async function restartGinlspClient(
  context: vscode.ExtensionContext,
  options: { notifyOnSuccess: boolean },
): Promise<void> {
  lspOutput?.appendLine("[gin] Restarting language server…");
  const previous = client;
  client = undefined;
  if (previous) {
    await previous.stop();
    const idx = context.subscriptions.indexOf(previous);
    if (idx >= 0) {
      context.subscriptions.splice(idx, 1);
    }
  }
  const channel = lspOutput ?? vscode.window.createOutputChannel("Gin Language Server");
  const next = createClient(channel);
  client = next;
  context.subscriptions.push(next);
  try {
    await next.start();
    lspOutput?.appendLine("[gin] Language server started.");
    if (options.notifyOnSuccess) {
      void vscode.window.showInformationMessage("Gin language server restarted.");
    }
  } catch (err: unknown) {
    const message = err instanceof Error ? err.message : String(err);
    void vscode.window.showErrorMessage(
      `Gin: could not restart ginlsp (${ginlspCommand()}). ${message}`,
    );
  }
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}
