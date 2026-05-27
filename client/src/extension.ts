import * as fs from 'fs';
import * as path from 'path';
import { ExtensionContext, workspace, window } from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

function resolveServerBinary(context: ExtensionContext): string | undefined {
    const override = process.env.URDF_LSP_BIN;
    if (override) {
        return fs.existsSync(override) ? override : undefined;
    }
    const release = context.asAbsolutePath(path.join('server', 'bin', 'urdf-lsp'));
    if (fs.existsSync(release)) return release;
    const debug = context.asAbsolutePath(path.join('server', 'target', 'debug', 'urdf-lsp'));
    if (fs.existsSync(debug)) return debug;
    return undefined;
}

export async function activate(context: ExtensionContext): Promise<void> {
    const serverBinary = resolveServerBinary(context);
    if (!serverBinary) {
        window.showErrorMessage(
            'urdf-lsp: server binary not found. Set URDF_LSP_BIN, or build the server ' +
            'with `cargo build --release --manifest-path server/Cargo.toml` and stage ' +
            'it at server/bin/urdf-lsp.',
        );
        return;
    }

    const serverOptions: ServerOptions = {
        command: serverBinary,
        args: [],
        transport: TransportKind.stdio,
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'urdf' }],
        synchronize: {
            fileEvents: workspace.createFileSystemWatcher('**/*.{urdf,xacro}'),
        },
    };

    const lc = new LanguageClient('urdfLsp', 'URDF Language Server', serverOptions, clientOptions);
    context.subscriptions.push(lc);
    client = lc;

    try {
        await lc.start();
    } catch (err) {
        client = undefined;
        window.showErrorMessage(
            `Failed to start urdf-lsp at ${serverBinary}: ${err instanceof Error ? err.message : String(err)}`,
        );
    }
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}
