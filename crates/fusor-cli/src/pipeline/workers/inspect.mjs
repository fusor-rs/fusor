// Compilation and import enumeration execute no Wasm or JavaScript startup.
import {readFile} from 'node:fs/promises';
const module = await WebAssembly.compile(await readFile(process.argv[1]));
process.stdout.write(JSON.stringify({pool: WebAssembly.Module.imports(module).some(entry => entry.name.includes('require_pool'))}));
