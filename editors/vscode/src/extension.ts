import * as vscode from 'vscode';
import {
  LanguageClient,
  type LanguageClientOptions,
  type ServerOptions,
} from 'vscode-languageclient/node';
import { ServerLifecycle, type Session } from './lifecycle';
import { checkExecutable, serverPath } from './server-path';

const defaultWatchPatterns = [
  '**/glsld.toml',
  '**/*.{glsl,vert,frag,geom,tesc,tese,comp,mesh,task,rgen,rint,rahit,rchit,rmiss,rcall,vs,fs,gs,vsh,fsh,ps,inc,h}',
];

let lifecycle: ServerLifecycle | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  if (!vscode.workspace.isTrusted) {
    return;
  }

  const output = vscode.window.createOutputChannel('glsld', { log: true });
  context.subscriptions.push(output);

  const manager = new ServerLifecycle(() => createSession(context, output));
  lifecycle = manager;

  const restart = async (): Promise<void> => {
    try {
      await manager.restart();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      output.appendLine(message);
      void vscode.window.showErrorMessage(`glsld could not start: ${message}`);
    }
  };

  context.subscriptions.push(
    vscode.commands.registerCommand('glsld.restart', restart),
    vscode.workspace.onDidChangeConfiguration(event => {
      if (
        event.affectsConfiguration('glsld.path') ||
        event.affectsConfiguration('glsld.watchPatterns')
      ) {
        void restart();
      }
    }),
  );
  await restart();
}

async function createSession(
  context: vscode.ExtensionContext,
  output: vscode.LogOutputChannel,
): Promise<Session> {
  const config = vscode.workspace.getConfiguration('glsld');
  const executable = serverPath(context.extensionPath, config.get<string>('path', ''));
  await checkExecutable(executable);

  const customPatterns = config.get<string[]>('watchPatterns', []);
  const patterns = new Set([...defaultWatchPatterns, ...customPatterns]);
  const watchers: vscode.FileSystemWatcher[] = [];

  try {
    for (const pattern of patterns) {
      watchers.push(vscode.workspace.createFileSystemWatcher(pattern));
    }

    const serverOptions: ServerOptions = { command: executable, args: ['--stdio'] };
    const clientOptions: LanguageClientOptions = {
      documentSelector: [{ scheme: 'file', language: 'glsl' }],
      outputChannel: output,
      traceOutputChannel: output,
      synchronize: { fileEvents: watchers },
      // An unset workspaceFolder synchronizes all workspace roots.
    };
    const client = new LanguageClient('glsld', 'glsld', serverOptions, clientOptions);

    return {
      start: () => client.start(),
      stop: async () => {
        try {
          await client.dispose();
        } finally {
          watchers.forEach(watcher => watcher.dispose());
        }
      },
    };
  } catch (error) {
    watchers.forEach(watcher => watcher.dispose());
    throw error;
  }
}

export async function deactivate(): Promise<void> {
  const manager = lifecycle;
  lifecycle = undefined;
  await manager?.stop();
}
