#!/usr/bin/env node
// Generates THIRD_PARTY_LICENSES.txt: the license texts of every Rust crate and npm package shipped in the app,
// shown in Settings > About > Licenses. Usage: pnpm notices [--check]
//   --check  fails when the committed file is out of date instead of rewriting it.
import { execSync } from 'node:child_process';
import { existsSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const output = join(root, 'THIRD_PARTY_LICENSES.txt');
const LICENSE_FILE = /^(licen[cs]e|copying|unlicense|notice|copyright)([-._].*)?$/i;
/** Standard texts (from SPDX) used when a package declares a license but ships no license file, in order of preference. */
const STANDARD = ['MIT', 'Apache-2.0', 'BSD-3-Clause', 'MPL-2.0'];

const run = (command) => execSync(command, { cwd: root, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
const normalize = (text) => text.replace(/\r\n?/g, '\n').replace(/[ \t]+$/gm, '').trim();

/** License texts found in a package directory, plus an explicitly declared license file. */
function licenseTexts(dir, declared) {
  const files = new Set(existsSync(dir) ? readdirSync(dir).filter((f) => LICENSE_FILE.test(f)) : []);
  if (declared) files.add(declared);
  return [...files]
    .sort()
    .map((f) => join(dir, f))
    .filter((p) => existsSync(p))
    .map((p) => normalize(readFileSync(p, 'utf8')))
    .filter(Boolean);
}

/** For a package that ships no license file: the standard text of one license its expression allows. */
function standardText(p) {
  if (/\bAND\b/.test(p.license)) return [];
  const allowed = p.license.split(/\s+OR\s+|\//).map((s) => s.replace(/[()]/g, '').trim());
  const id = STANDARD.find((s) => allowed.includes(s));
  if (!id) return [];
  const holders = p.authors.length > 0 ? p.authors.join(', ') : `the ${p.name} authors`;
  const text = readFileSync(join(root, 'scripts', 'licenses', `${id}.txt`), 'utf8')
    .replace(/^Copyright \(c\) <year> <[^>]+>\.?/m, `Copyright (c) ${holders}`);
  return [`${id} (standard text; the package ships no license file)\n\n${normalize(text)}`];
}

const withTexts = (p) => (p.texts.length > 0 ? p : { ...p, texts: standardText(p) });
const stripEmail = (author) => author.replace(/\s*<[^>]*>/g, '').trim();

/** Crates linked into the app: normal dependencies reachable from the workspace, for every target platform. */
function rustPackages() {
  const metadata = JSON.parse(run('cargo metadata --format-version 1 --locked'));
  const workspace = new Set(metadata.workspace_members);
  const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
  const packages = new Map(metadata.packages.map((p) => [p.id, p]));
  const reached = new Set();
  const stack = [...workspace];
  while (stack.length > 0) {
    const id = stack.pop();
    if (reached.has(id)) continue;
    reached.add(id);
    for (const dep of nodes.get(id).deps) {
      if (dep.dep_kinds.some((k) => k.kind === null)) stack.push(dep.pkg);
    }
  }
  return [...reached]
    .filter((id) => !workspace.has(id))
    .map((id) => packages.get(id))
    .map((p) => ({
      name: p.name,
      version: p.version,
      license: p.license ?? 'see license file',
      authors: p.authors.map(stripEmail),
      source: p.repository ?? `https://crates.io/crates/${p.name}`,
      texts: licenseTexts(dirname(p.manifest_path), p.license_file),
    }))
    .map(withTexts);
}

/** npm packages bundled into the frontend: production dependencies only. */
function npmPackages() {
  const byLicense = JSON.parse(run('pnpm licenses list --prod --json'));
  return Object.values(byLicense).flat().flatMap((p) =>
    p.paths.map((path) => {
      const manifest = JSON.parse(readFileSync(join(path, 'package.json'), 'utf8'));
      const repository = typeof manifest.repository === 'string' ? manifest.repository : manifest.repository?.url;
      return withTexts({
        name: p.name,
        version: manifest.version,
        license: p.license,
        authors: [manifest.author].filter(Boolean).map((a) => stripEmail(typeof a === 'string' ? a : a.name)),
        source: p.homepage ?? repository ?? `https://www.npmjs.com/package/${p.name}`,
        texts: licenseTexts(path, null),
      });
    }),
  );
}

/** Assets copied into the source tree rather than installed as packages. */
const VENDORED = [
  {
    name: 'Devicon (MySQL and SQLite marks in src/components/engineLogos.ts)',
    version: '',
    license: 'MIT',
    source: 'https://devicon.dev',
    texts: [normalize(`The MIT License (MIT)

Copyright (c) 2015 konpa

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.`)],
  },
  {
    name: 'Simple Icons (MariaDB mark in src/components/engineLogos.ts)',
    version: '16.37.0',
    license: 'CC0-1.0',
    source: 'https://simpleicons.org',
    texts: ['Released under CC0 1.0 Universal (public domain dedication); no attribution required.'],
  },
];

const RULE = '-'.repeat(80);

/** Packages sharing identical license texts are listed once, above that text. */
function section(title, packages) {
  const groups = new Map();
  const missing = [];
  for (const p of packages.sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version, undefined, { numeric: true }))) {
    if (p.texts.length === 0) missing.push(p);
    const key = p.texts.join('\n\n\f\n\n');
    const group = groups.get(key) ?? { packages: [], texts: p.texts };
    group.packages.push(p);
    groups.set(key, group);
  }
  const lines = [`${'='.repeat(80)}\n${title} (${packages.length})\n${'='.repeat(80)}`];
  for (const { packages: members, texts } of groups.values()) {
    if (texts.length === 0) continue;
    lines.push(RULE);
    for (const p of members) lines.push(`${p.name}${p.version ? ` ${p.version}` : ''} (${p.license}) ${p.source}`);
    lines.push(RULE, texts.join(`\n\n${'- '.repeat(40).trim()}\n\n`), '');
  }
  return { text: lines.join('\n'), missing };
}

const sections = [
  section('Rust crates', rustPackages()),
  section('npm packages', npmPackages()),
  section('Vendored assets', VENDORED),
];
const missing = sections.flatMap((s) => s.missing);
if (missing.length > 0) {
  console.error(`No license text found for:\n${missing.map((p) => `  ${p.name} ${p.version} (${p.license})`).join('\n')}`);
  process.exit(1);
}

const content = `Schemata - third-party software notices

Schemata is licensed under the MIT License (see LICENSE). It includes the
third-party software listed below, each under its own license. Where a package
offers a choice of licenses, Schemata uses it under any one of them; the texts
the package ships are reproduced as published.

Generated by scripts/third-party-licenses.mjs; do not edit by hand.

${sections.map((s) => s.text).join('\n')}`;

if (process.argv.includes('--check')) {
  const current = existsSync(output) ? readFileSync(output, 'utf8') : '';
  if (current !== content) {
    console.error('THIRD_PARTY_LICENSES.txt is out of date; run: node scripts/third-party-licenses.mjs');
    process.exit(1);
  }
} else {
  writeFileSync(output, content);
  console.log(`Wrote ${output}`);
}
