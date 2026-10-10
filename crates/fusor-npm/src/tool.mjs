// Internal host tooling. This process never installs dependencies or accesses a
// registry. Import resolution/transpilation belongs to esbuild; no library API
// inspection or generated Rust bindings are involved.
import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';

try {
  const [rootArg, entryArg, modulesArg, publicPath, esbuildVersion, release] = process.argv.slice(2);
  if (Number(process.versions.node.split('.')[0]) < 22) throw Error('JavaScript modules require Node.js 22 or newer');
  const root = fs.realpathSync(rootArg);
  const entry = path.resolve(entryArg);
  const output = path.dirname(entry);
  const read = file => JSON.parse(fs.readFileSync(file, 'utf8'));
  const manifestFile = path.join(root, 'package.json');
  const lockFile = path.join(root, 'package-lock.json');
  if (!fs.existsSync(manifestFile) || !fs.existsSync(lockFile)) {
    throw Error(`JavaScript modules require package.json and a committed package-lock.json at the application root. Install esbuild@${esbuildVersion} with npm, then run npm ci before building.`);
  }
  const manifest = read(manifestFile);
  const lock = read(lockFile);
  const modules = read(modulesArg);
  const inputs = new Set([manifestFile, lockFile]);
  if (lock.lockfileVersion !== 3 || !lock.packages?.['']) throw Error('Use a committed npm package-lock.json version 3 and run npm ci before building.');
  for (const kind of ['dependencies', 'devDependencies', 'optionalDependencies']) {
    const ordered = object => JSON.stringify(Object.entries(object || {}).sort());
    if (ordered(manifest[kind]) !== ordered(lock.packages[''][kind])) throw Error(`package.json ${kind} differs from package-lock.json; update the lockfile and run npm ci`);
  }
  const checkedPackages = new Set();
  function installed(file) {
    let directory = path.dirname(file);
    while (directory !== root && directory !== path.dirname(directory)) {
      const relative = path.relative(root, directory).split(path.sep).join('/');
      if (/(?:^|\/)node_modules\/(?:@[^/]+\/)?[^/]+$/.test(relative)) {
        const packageFile = path.join(directory, 'package.json');
        if (checkedPackages.has(packageFile)) return;
        const locked = lock.packages[relative];
        if (locked?.link || fs.realpathSync(directory) !== directory) throw Error('Linked/workspace npm packages are not supported; use an app-local locked installation');
        const data = read(packageFile);
        if (!locked || data.version !== locked.version) throw Error(`Installed ${data.name || directory}@${data.version} does not match package-lock.json; run npm ci`);
        checkedPackages.add(packageFile);
        inputs.add(packageFile);
        return;
      }
      directory = path.dirname(directory);
    }
    throw Error(`Expected an app-local locked installed npm package: ${file}`);
  }
  // Manifests affect exports/browser resolution even when their package redirects
  // to another package without loading a file of its own. Track installed locked
  // manifests explicitly; absent optional platform packages are expected.
  for (const relative of Object.keys(lock.packages)) {
    if (!relative.startsWith('node_modules/')) continue;
    const packageFile = path.join(root, relative, 'package.json');
    if (fs.existsSync(packageFile)) installed(packageFile);
  }
  const require = createRequire(manifestFile);
  let esbuild;
  try {
    const toolFile = require.resolve('esbuild');
    installed(toolFile);
    inputs.add(toolFile);
    esbuild = require('esbuild');
  } catch (error) {
    throw Error(`Missing or invalid installed JavaScript tooling: ${error.message}. Pin esbuild@${esbuildVersion} and run npm ci in this application.`);
  }
  if (esbuild.version !== esbuildVersion) throw Error(`This bundler supports esbuild ${esbuildVersion}; installed ${esbuild.version}. Pin esbuild@${esbuildVersion} and update the lockfile.`);
  const binaryPackage = `@esbuild/${process.platform}-${process.arch}`;
  try {
    const binary = require.resolve(`${binaryPackage}/package.json`);
    installed(binary);
    inputs.add(binary);
  } catch (error) {
    throw Error(`Missing or invalid esbuild platform package ${binaryPackage}; run npm ci (${error.message})`);
  }

  const byPath = new Map();
  const ids = new Set();
  for (const module of modules) {
    if (!module.id || typeof module.path !== 'string' || typeof module.source !== 'string' || ids.has(module.id)) throw Error('Invalid or duplicate component JavaScript module metadata');
    ids.add(module.id);
    module.path = path.resolve(module.path);
    module.source = path.resolve(module.source);
    inputs.add(module.source);
    if (module.inline) byPath.set(module.path, module);
  }
  const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  function vlq(value) {
    let number = value < 0 ? (-value * 2) + 1 : value * 2;
    let result = '';
    do {
      let digit = number % 32;
      number = Math.floor(number / 32);
      if (number) digit += 32;
      result += alphabet[digit];
    } while (number);
    return result;
  }
  function inlineSource(module) {
    const contents = fs.readFileSync(module.path, 'utf8');
    const lines = contents.split('\n').length;
    const column = Math.max(0, module.column - 1);
    const first = `AA${vlq(Math.max(0, module.line - 1))}${vlq(column)}`;
    const second = `AAC${vlq(-column)}`;
    const map = {
      version: 3,
      sources: [module.source],
      sourcesContent: [fs.readFileSync(module.source, 'utf8')],
      names: [],
      mappings: [first, ...Array.from({ length: lines - 1 }, (_, index) => index ? 'AACA' : second)].join(';'),
    };
    return `${contents}\n//# sourceMappingURL=data:application/json;base64,${Buffer.from(JSON.stringify(map)).toString('base64')}\n`;
  }
  const imports = modules.map((module, index) => `import * as component${index} from ${JSON.stringify(module.path)};`).join('\n');
  const registry = `globalThis[Symbol.for('fusor.javascript.modules.v1')] = new Map([${modules.map((module, index) => `[${JSON.stringify(module.id)},component${index}]`).join(',')}]);`;
  const wrapper = `${imports}\n${registry}\nexport * from ${JSON.stringify(entry)};\nexport { default } from ${JSON.stringify(entry)};`;
  let result;
  try {
    result = await esbuild.build({
      entryPoints: [{ in: 'fusor-browser-entry', out: path.basename(entry, '.js') }],
      absWorkingDir: root, outdir: output, bundle: true, splitting: true,
      publicPath,
      format: 'esm', platform: 'browser', target: 'es2022', write: false,
      allowOverwrite: true, minify: release === '--release', treeShaking: true,
      sourcemap: release === '--release' ? false : 'linked', sourcesContent: true,
      metafile: true, logLevel: 'silent', nodePaths: [path.join(root, 'node_modules')],
      chunkNames: 'chunks/[name]-[hash]', assetNames: 'assets/[name]-[hash]',
      loader: Object.fromEntries(['.png', '.jpg', '.jpeg', '.svg', '.gif', '.webp', '.avif', '.ico', '.woff', '.woff2', '.ttf', '.otf', '.eot', '.mp4', '.webm', '.ogg', '.wav', '.mp3', '.pdf', '.wasm', '.bin'].map(extension => [extension, 'file'])),
      plugins: [{ name: 'fusor-modules', setup(build) {
        build.onResolve({ filter: /^fusor-browser-entry$/ }, () => ({ path: 'fusor-browser-entry', namespace: 'fusor' }));
        build.onLoad({ filter: /.*/, namespace: 'fusor' }, () => ({ contents: wrapper, loader: 'js', resolveDir: root }));
        build.onLoad({ filter: /.*/, namespace: 'file' }, args => {
          if (args.path.split(path.sep).includes('node_modules')) installed(args.path);
          const module = byPath.get(args.path);
          if (module) return { contents: inlineSource(module), loader: 'js', resolveDir: path.dirname(module.source) };
        });
      } }],
    });
  } catch (error) {
    if (!error.errors) throw error;
    for (const diagnostic of error.errors) {
      const location = diagnostic.location;
      const module = location && byPath.get(path.resolve(root, location.file));
      if (module) diagnostic.location = { ...location, file: module.source, line: location.line + module.line - 1, column: location.column + (location.line === 1 ? module.column - 1 : 0) };
    }
    throw Error(esbuild.formatMessagesSync(error.errors, { kind: 'error', color: false }).join('\n'));
  }
  for (const warning of esbuild.formatMessagesSync(result.warnings, { kind: 'warning', color: false })) process.stderr.write(warning);
  for (const file of Object.keys(result.metafile.inputs)) {
    if (file.startsWith('fusor:')) continue;
    const absolute = path.resolve(root, file);
    if (byPath.has(absolute) || absolute === output || absolute.startsWith(output + path.sep)) continue;
    inputs.add(absolute);
  }
  const styles = [];
  for (const file of result.outputFiles) {
    fs.mkdirSync(path.dirname(file.path), { recursive: true });
    fs.writeFileSync(file.path, file.contents);
    if (file.path.endsWith('.css')) styles.push(path.relative(output, file.path).split(path.sep).join('/'));
  }
  fs.writeFileSync(path.join(output, 'javascript-bundle.json'), JSON.stringify({ inputs: [...inputs].sort(), styles: styles.sort() }, null, 2));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
