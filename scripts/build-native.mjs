import { spawnSync } from 'node:child_process';
import { access, copyFile, mkdir, readdir, rm } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export async function buildNative() {
  const platform = process.platform;
  const bundle = {
    win32: { output: 'windows' },
    linux: { format: 'appimage', directory: 'appimage', extension: '.AppImage', output: 'linux' },
    darwin: { format: 'dmg', directory: 'dmg', extension: '.dmg', output: 'macos' },
  }[platform];

  if (!bundle) {
    throw new Error(`Native packaging is not supported on ${platform}.`);
  }
  if (process.env.CARGO_BUILD_TARGET) {
    throw new Error(
      'Native packaging requires the host target. Unset CARGO_BUILD_TARGET before running this build.',
    );
  }

  const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
  const target = join(root, 'target', 'package-native', platform);
  const bundleDirectory = platform === 'win32' ? null : join(target, 'release', 'bundle', bundle.directory);
  const destination = join(root, 'package', bundle.output);
  const env = { ...process.env, CARGO_TARGET_DIR: target };
  const command = platform === 'win32' ? 'pnpm.cmd' : 'pnpm';
  let innoCompiler;
  if (platform === 'win32') {
    const onPath = spawnSync('where.exe', ['ISCC.exe'], { encoding: 'utf8' });
    innoCompiler = onPath.status === 0 ? onPath.stdout.trim().split(/\r?\n/)[0] : undefined;
    if (!innoCompiler) {
      const installed = join(process.env['ProgramFiles(x86)'] ?? 'C:\\Program Files (x86)', 'Inno Setup 6', 'ISCC.exe');
      try {
        await access(installed);
        innoCompiler = installed;
      } catch {
        throw new Error('Install Inno Setup 6 (6.6 or newer) and add ISCC.exe to PATH, or use its default installation directory.');
      }
    }
  }

  for (const args of [
    ['install', '--frozen-lockfile'],
    ['exec', 'tauri', 'build', ...(platform === 'win32' ? ['--no-bundle'] : ['--bundles', bundle.format]), '--', '--locked'],
  ]) {
    if (args[0] === 'exec' && bundleDirectory) {
      // Only remove this script's bundle output, leaving normal Cargo builds untouched.
      await rm(bundleDirectory, { recursive: true, force: true });
    }
    // Every token is internal and fixed; pass no argument array to the Windows shell.
    const invocation = platform === 'win32' ? [`${command} ${args.join(' ')}`] : [command, args];
    const result = spawnSync(...invocation, {
      cwd: root,
      env,
      stdio: 'inherit',
      shell: platform === 'win32',
    });
    if (result.error) {
      throw new Error(`Failed to run pnpm ${args.join(' ')}.`, { cause: result.error });
    }
    if (result.status !== 0) {
      throw new Error(
        `pnpm ${args.join(' ')} failed (${result.signal ? `signal ${result.signal}` : `exit ${result.status}`}).`,
      );
    }
  }

  if (platform === 'win32') {
    const result = spawnSync(innoCompiler, [join(root, 'setup.iss')], {
      cwd: root,
      env,
      stdio: 'inherit',
    });
    if (result.error) throw new Error('Failed to start Inno Setup.', { cause: result.error });
    if (result.status !== 0) throw new Error(`Inno Setup failed (exit ${result.status}).`);
    return;
  }

  let entries;
  try {
    entries = await readdir(bundleDirectory, { withFileTypes: true });
  } catch (error) {
    throw new Error(`Native build produced no readable installer directory: ${bundleDirectory}`, {
      cause: error,
    });
  }
  const installers = entries
    .filter((entry) => entry.isFile() && entry.name.endsWith(bundle.extension))
    .map((entry) => entry.name)
    .sort();
  if (installers.length === 0) {
    throw new Error(`Native build produced no ${bundle.extension} installer in ${bundleDirectory}.`);
  }

  await mkdir(destination, { recursive: true });
  const outputs = [];
  for (const installer of installers) {
    const output = join(destination, installer);
    await copyFile(join(bundleDirectory, installer), output);
    outputs.push(output);
    console.log(`Installer: ${output}`);
  }
  return outputs;
}
