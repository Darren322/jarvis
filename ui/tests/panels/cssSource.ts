/**
 * The text of a stylesheet in `src/`, for tests of rules jsdom cannot apply
 * (`::after`, media queries). Vitest turns a CSS import into an empty string,
 * `?raw` included, so the file is read from disk with Node's own `fs` (the UI
 * has no `@types/node`, hence the small typed view of it).
 */

interface NodeFs {
  readFileSync(path: URL, encoding: 'utf8'): string;
}

interface NodeProcess {
  getBuiltinModule(id: 'node:fs'): NodeFs;
}

/**
 * This file's URL, in a variable: Vite rewrites `new URL(<literal>, import.meta.url)`
 * into an asset import, which is not wanted here.
 */
const HERE = import.meta.url;

/** The stylesheet at `path`, relative to `ui/src/`. */
export function cssSource(path: string): string {
  const { process } = globalThis as unknown as { process: NodeProcess };
  return process.getBuiltinModule('node:fs').readFileSync(new URL(`../../src/${path}`, HERE), 'utf8');
}
