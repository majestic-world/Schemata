import { spawnSync } from 'node:child_process';
import { createReadStream, createWriteStream } from 'node:fs';
import { chmod, copyFile, mkdir, mkdtemp, open, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, relative, resolve } from 'node:path';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath } from 'node:url';
import { createGzip } from 'node:zlib';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const xmlHeader = '<?xml version="1.0" encoding="utf-8"?>\n';

function escapeXml(value) {
  const entities = { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&apos;' };
  return String(value).replace(/[&<>"']/g, (character) => entities[character]);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: 'inherit', ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} failed (${result.signal ? `signal ${result.signal}` : `exit ${result.status}`}).`);
  }
}

async function main() {
  const config = JSON.parse(await readFile(join(root, 'src-tauri/tauri.conf.json'), 'utf8'));
  const { productName: product, version, identifier } = config;
  const binary = 'schematadb';
  const minimum = '11.0';
  const output = '/out';
  await mkdir(output, { recursive: true });

  // The frontend is embedded in the executable, like the native Tauri build.
  // Refuse unsupported additions rather than silently omit runtime resources.
  const bundle = config.bundle ?? {};
  if (['resources', 'externalBin', 'macOS'].some((key) => Object.keys(bundle[key] ?? {}).length > 0)) {
    throw new Error('Update the cross-packager for the new Tauri bundle resources/macOS settings.');
  }

  const work = await mkdtemp(join(tmpdir(), 'schematadb-pkg-'));
  try {
    const payloadRoot = join(work, 'root');
    const app = join(payloadRoot, 'Applications', `${product}.app`);
    const contents = join(app, 'Contents');
    await mkdir(join(contents, 'MacOS'), { recursive: true });
    await mkdir(join(contents, 'Resources'));
    const executable = join(contents, 'MacOS', binary);
    await copyFile(join(root, 'target/aarch64-apple-darwin/release', binary), executable);
    await chmod(executable, 0o755);
    await copyFile(join(root, 'src-tauri/icons/icon.icns'), join(contents, 'Resources/icon.icns'));
    const metadata = {
      CFBundleDevelopmentRegion: 'English',
      CFBundleDisplayName: product,
      CFBundleExecutable: binary,
      CFBundleIdentifier: identifier,
      CFBundleInfoDictionaryVersion: '6.0',
      CFBundleName: product,
      CFBundlePackageType: 'APPL',
      CFBundleShortVersionString: version,
      CFBundleVersion: version,
      CFBundleIconFile: 'icon.icns',
      LSMinimumSystemVersion: minimum,
      NSHighResolutionCapable: true,
    };
    const dictionary = Object.entries(metadata).map(([key, value]) =>
      `  <key>${escapeXml(key)}</key>\n  ${typeof value === 'boolean' ? `<${value}/>` : `<string>${escapeXml(value)}</string>`}`,
    ).join('\n');
    await writeFile(join(contents, 'Info.plist'), `${xmlHeader}<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n<plist version="1.0">\n<dict>\n${dictionary}\n</dict>\n</plist>\n`);
    // Apple Silicon requires an executable signature, even for local builds.
    // This is ad-hoc signing, not Developer ID signing or notarization.
    run('rcodesign', ['sign', app]);

    const flat = join(work, 'flat');
    const component = join(flat, 'base.pkg');
    await mkdir(component, { recursive: true });
    const entries = await readdir(payloadRoot, { recursive: true, withFileTypes: true });
    const paths = ['.', ...entries.map((entry) => relative(payloadRoot, join(entry.parentPath, entry.name))).sort()];
    let bytes = 0;
    for (const entry of entries) {
      if (entry.isFile()) bytes += (await stat(join(entry.parentPath, entry.name))).size;
    }
    const size = Math.ceil(bytes / 1024);
    const cpioPath = join(work, 'Payload.cpio');
    const cpio = await open(cpioPath, 'w');
    try {
      run('cpio', ['--create', '--format=odc', '--owner=0:80', '--null'], {
        cwd: payloadRoot,
        input: Buffer.from(`${paths.join('\0')}\0`),
        stdio: ['pipe', cpio.fd, 'inherit'],
      });
    } finally {
      await cpio.close();
    }
    await pipeline(createReadStream(cpioPath), createGzip(), createWriteStream(join(component, 'Payload')));
    run('mkbom', ['-u', '0', '-g', '80', payloadRoot, join(component, 'Bom')]);

    const packageId = escapeXml(`${identifier}.installer`);
    const productXml = escapeXml(product);
    const versionXml = escapeXml(version);
    const identifierXml = escapeXml(identifier);
    await writeFile(join(component, 'PackageInfo'), `${xmlHeader}<pkg-info format-version="2" identifier="${packageId}" version="${versionXml}" install-location="/" auth="root">
  <payload installKBytes="${size}" numberOfFiles="${paths.length}"/>
  <bundle-version>
    <bundle id="${identifierXml}" CFBundleIdentifier="${identifierXml}" path="./Applications/${productXml}.app" CFBundleVersion="${versionXml}"/>
  </bundle-version>
</pkg-info>
`);
    await writeFile(join(flat, 'Distribution'), `${xmlHeader}<installer-gui-script minSpecVersion="2">
  <title>${productXml}</title>
  <options customize="never" require-scripts="false" hostArchitectures="arm64"/>
  <domains enable_anywhere="false" enable_currentUserHome="false" enable_localSystem="true"/>
  <allowed-os-versions>
    <os-version min="${minimum}"/>
  </allowed-os-versions>
  <choices-outline>
    <line choice="default"/>
  </choices-outline>
  <choice id="default" visible="false" title="${productXml}">
    <pkg-ref id="${packageId}"/>
  </choice>
  <pkg-ref id="${packageId}" version="${versionXml}" installKBytes="${size}" auth="Root">#base.pkg</pkg-ref>
</installer-gui-script>
`);
    const installer = join(output, `${product}_${version}_aarch64.pkg`);
    run('bsdtar', [
      '--format=xar', '--options=xar:compression=none',
      '-cf', installer, '-C', flat, 'Distribution', 'base.pkg',
    ]);
    console.log(installer);
  } finally {
    await rm(work, { recursive: true, force: true });
  }
}

try {
  await main();
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
