import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";

const counter = ".fusor/asset-hook-count";
mkdirSync(".fusor", { recursive: true });
const count = (existsSync(counter) ? Number(readFileSync(counter, "utf8")) : 0) + 1;
writeFileSync(counter, String(count));
writeFileSync("public/asset-hook.css", `:root { --asset-build: ${count}; }`);
