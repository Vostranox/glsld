import { readFile, access } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { spawnSync } from 'node:child_process';
import path from 'node:path';

const supportedTargets = [
  'linux-x64',
  'linux-arm64',
  'darwin-x64',
  'darwin-arm64',
  'win32-x64',
  'win32-arm64',
];

function detectBinaryTarget(bytes) {
  // ELF: the class, byte order, and machine fields identify a Linux target.
  if (bytes.length >= 20 && bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
    if (bytes[4] !== 2 || bytes[5] !== 1) {
      throw new Error('Only 64-bit little-endian ELF binaries are supported.');
    }

    const machine = bytes.readUInt16LE(18);
    if (machine === 62) {
      return 'linux-x64';
    }
    if (machine === 183) {
      return 'linux-arm64';
    }
  }

  // Mach-O: recognize the 64-bit magic and CPU type used by macOS.
  if (bytes.length >= 8 && bytes.readUInt32LE(0) === 0xfeedfacf) {
    const cpu = bytes.readUInt32LE(4);
    if (cpu === 0x1000007) {
      return 'darwin-x64';
    }
    if (cpu === 0x100000c) {
      return 'darwin-arm64';
    }
  }

  // PE: follow the DOS header's offset to the Windows signature and machine type.
  if (bytes.length >= 64 && bytes.toString('ascii', 0, 2) === 'MZ') {
    const peOffset = bytes.readUInt32LE(60);
    if (peOffset + 6 <= bytes.length && bytes.readUInt32LE(peOffset) === 0x4550) {
      const machine = bytes.readUInt16LE(peOffset + 4);
      if (machine === 0x8664) {
        return 'win32-x64';
      }
      if (machine === 0xaa64) {
        return 'win32-arm64';
      }
    }
  }

  throw new Error('Unrecognized glsld native executable format or architecture.');
}

function readRequestedTarget(cliArguments) {
  const target = cliArguments[cliArguments.indexOf('--target') + 1];

  if (!cliArguments.includes('--target') || !supportedTargets.includes(target)) {
    throw new Error(
      `Choose a native VSIX target: npm run package -- --target <${supportedTargets.join('|')}>`,
    );
  }

  return target;
}

async function validateNativeFiles(target) {
  const binary = `bin/${target.startsWith('win32-') ? 'glsld.exe' : 'glsld'}`;
  const actualTarget = detectBinaryTarget(await readFile(binary));

  if (actualTarget !== target) {
    throw new Error(`${binary} targets ${actualTarget}, but the requested VSIX targets ${target}.`);
  }

  for (const notice of ['LICENSE', 'THIRD_PARTY_NOTICES.md', 'third-party']) {
    await access(notice);
  }
}

async function main() {
  const cliArguments = process.argv.slice(2);
  const target = readRequestedTarget(cliArguments);
  await validateNativeFiles(target);

  const require = createRequire(import.meta.url);
  const vsce = path.join(path.dirname(require.resolve('@vscode/vsce/package.json')), 'vsce');
  const result = spawnSync(
    process.execPath,
    [vsce, 'package', '--no-dependencies', ...cliArguments],
    { stdio: 'inherit' },
  );

  if (result.error) {
    throw result.error;
  }

  process.exit(result.status ?? 1);
}

await main();
