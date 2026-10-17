import { build } from 'esbuild';
import { readFile, readdir, realpath, writeFile } from 'node:fs/promises';
import path from 'node:path';

async function findOwningPackage(input) {
  let directory = path.dirname(input);

  while (true) {
    try {
      const manifest = JSON.parse(await readFile(path.join(directory, 'package.json'), 'utf8'));

      if (manifest.name && manifest.version) {
        return { directory, manifest };
      }
    } catch (error) {
      if (error.code !== 'ENOENT') {
        throw error;
      }
    }

    const parent = path.dirname(directory);
    if (parent === directory) {
      throw new Error(`Cannot find package metadata for ${input}`);
    }

    directory = parent;
  }
}

async function readPackageLicenses(directory, packageId) {
  const files = (await readdir(directory))
    .filter(name => /^(licen[cs]e|copying|notice)(\.|$)/i.test(name))
    .sort();

  if (files.length === 0) {
    throw new Error(`No license found for bundled dependency ${packageId}`);
  }

  const licenses = await Promise.all(
    files.map(name => readFile(path.join(directory, name), 'utf8')),
  );

  if (licenses.some(license => !license.trim())) {
    throw new Error(`Empty license found for bundled dependency ${packageId}`);
  }

  return licenses.join('\n');
}

async function collectDependencyLicenses(inputs, projectRoot) {
  const root = await realpath(projectRoot);
  const packages = new Map();

  for (const input of inputs) {
    const file = await realpath(path.resolve(root, input));
    const { directory, manifest } = await findOwningPackage(file);

    // Symlinked dependencies can appear outside node_modules in esbuild's inputs.
    if (directory === root) {
      if (path.relative(root, file).split(path.sep).includes('node_modules')) {
        throw new Error(`Cannot find package metadata for ${input}`);
      }

      continue;
    }

    const packageId = `${manifest.name}@${manifest.version}`;
    if (packages.has(packageId)) {
      continue;
    }

    const licenseText = await readPackageLicenses(directory, packageId);
    packages.set(packageId, `${packageId} (${manifest.license})\n${licenseText}`);
  }

  return [...packages]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([, license]) => license);
}

async function main() {
  const result = await build({
    entryPoints: ['src/extension.ts'],
    outfile: 'dist/extension.js',
    bundle: true,
    platform: 'node',
    target: 'node20',
    format: 'cjs',
    external: ['vscode'],
    legalComments: 'inline',
    metafile: true,
  });

  const inputs = Object.keys(result.metafile.inputs);
  const licenses = await collectDependencyLicenses(inputs, process.cwd());

  if (licenses.length === 0) {
    throw new Error('No bundled dependency licenses were collected');
  }

  const heading = 'JavaScript dependencies bundled in glsld for VS Code\n\n';
  const separator = '\n' + '='.repeat(72) + '\n\n';
  await writeFile('THIRD_PARTY_LICENSES.txt', heading + licenses.join(separator));
}

await main();
