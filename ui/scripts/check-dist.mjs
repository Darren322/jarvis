// Fails if dist/index.html has a src or href starting with "/" (would break file://).
import { readFileSync } from 'node:fs';

const html = readFileSync(new URL('../dist/index.html', import.meta.url), 'utf8');
const absolute = [...html.matchAll(/\b(?:src|href)\s*=\s*["'](\/[^"']*)["']/g)].map((m) => m[1]);
if (absolute.length > 0) {
  console.error(`dist/index.html has root-absolute paths: ${absolute.join(', ')}`);
  process.exit(1);
}
console.log('dist/index.html asset paths are relative');
