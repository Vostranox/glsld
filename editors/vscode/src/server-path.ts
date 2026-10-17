import path from 'node:path';
import { access, stat } from 'node:fs/promises';
import { constants } from 'node:fs';

export function serverPath(
  extensionPath: string,
  override: string,
  platform = process.platform,
): string {
  const paths = platform === 'win32' ? path.win32 : path.posix;
  if (override !== '') {
    if (!paths.isAbsolute(override)) {
      throw new Error('glsld.path must be an absolute executable path on the extension host.');
    }
    return override;
  }

  const executable = platform === 'win32' ? 'glsld.exe' : 'glsld';
  return paths.join(extensionPath, 'bin', executable);
}

export async function checkExecutable(executable: string): Promise<void> {
  try {
    if (!(await stat(executable)).isFile()) {
      throw new Error('not a file');
    }
    await access(executable, process.platform === 'win32' ? constants.F_OK : constants.X_OK);
  } catch {
    throw new Error(
      `Cannot run ${executable}. Install the glsld extension for this platform ` +
      'or set glsld.path to an executable built for this extension host.',
    );
  }
}
