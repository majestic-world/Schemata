import { spawnSync } from 'node:child_process';
import { mkdirSync, readdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildNative } from './build-native.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
process.chdir(root);

try {
  const target = process.argv[2];
  if (target === 'native') {
    await buildNative();
  } else if (target === 'linux' || target === 'mac') {
    const destination = resolve('package', target === 'mac' ? 'macos' : 'linux');
    const extension = target === 'mac' ? '.pkg' : '.AppImage';
    mkdirSync(destination, { recursive: true });
    // Both containers run x86_64 build tools. OSXCross targets Apple ARM64.
    const result = spawnSync('docker', [
      'build', '--platform', 'linux/amd64', '--target', target,
      '--output', `type=local,dest=${destination}`, '--progress', 'plain', '.',
    ], { stdio: 'inherit' });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(`Docker ${target} build failed (exit ${result.status}).`);
    const packages = readdirSync(destination).filter((name) => name.endsWith(extension));
    if (packages.length === 0) throw new Error(`No ${extension} installer exported to ${destination}.`);
    for (const name of packages) console.log(resolve(destination, name));
  } else {
    throw new Error('Usage: node scripts/build.mjs native|linux|mac');
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
