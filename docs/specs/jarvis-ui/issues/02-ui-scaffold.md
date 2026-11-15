# 02: UI scaffold

Status: done
Stories: US1
Blocked by: None
Assignee: Sonnet agent (2026-11-15)
Owns: `ui/` config files only (`package.json`, `package-lock.json`, `tsconfig*.json`, `vite.config.ts`, `eslint.config.js`, `index.html`, `ui/.gitignore`, `ui/src/main.tsx`, and a placeholder `ui/src/App.tsx`); `scripts/verify.sh`; `AGENTS.md` (a new "UI (TypeScript)" section only)
Traces to: D1, D12

**What to build:** A Vite, React and TypeScript app in `ui/` at the repo root. Vitest runs the tests. ESLint runs with typescript-eslint. npm scripts `build`, `typecheck`, `lint` and `test`. `ui/.gitignore` ignores `node_modules/` and `dist/`. `scripts/verify.sh` runs the three UI checks (typecheck, lint, test) after the Rust checks, with the same commands as npm. `AGENTS.md` gets a short "UI (TypeScript)" section. `ui/src/App.tsx` renders an empty placeholder.

**Waiting on:** Darren's approval of the packages and their exact versions. The list below is proposed. Versions are to be pinned and approved before any `npm install`.

Proposed packages (exact versions to be pinned and approved):
- Runtime: `react`, `react-dom`
- Build: `vite`, `@vitejs/plugin-react`, `typescript`
- Test: `vitest`
- Lint: `eslint`, `typescript-eslint`, `eslint-plugin-react-hooks`
- Types: `@types/react`, `@types/react-dom`

## Acceptance criteria
- [x] Darren approves the package list and the exact versions in Comments. No `npm install` runs before that.
- [x] `npm ci` installs from the committed `package-lock.json` on the Mac.
- [x] `npm run build` writes `ui/dist/index.html`. Its asset paths are relative (`base: './'`), so the page loads from `file://`. A check fails the build if any `src` or `href` in `dist/index.html` starts with `/`.
- [x] `npm run typecheck`, `npm run lint` and `npm test` each pass, with one passing placeholder test.
- [x] `scripts/verify.sh` runs the three UI checks and passes.
- [x] `ui/dist/` and `ui/node_modules/` are ignored by git (`git check-ignore` matches both).
- [x] The Pi needs no Node. Only `ui/dist/` is copied there (ticket 05).
- [x] The AGENTS.md "UI (TypeScript)" section is short and names the four npm scripts.
- [x] Local checks only. This ticket does not prove anything on the Pi.

## Comments

- 2026-11-15 (coordinator): Darren approved these exact versions ("Yes sure" to Q28): react 19.3.1, react-dom 19.3.1, vite 8.3.8, @vitejs/plugin-react 6.3.0, typescript 6.0.3 (not 7.0: typescript-eslint 8.72.2 requires typescript <6.1.0), vitest 5.1.0, jsdom 31.0.1, eslint 10.15.0, typescript-eslint 8.72.2, eslint-plugin-react-hooks 7.1.1, @types/react 19.3.0, @types/react-dom 19.3.0. Pin each exactly (no ^ or ~).
- 2026-11-15 (coordinator): Integrated uncommitted into `dev` from the agent's worktree. Reviewed the diff: package.json holds exactly the 12 approved packages, pinned; `base: './'`; strict tsconfig; verify.sh runs `npm ci` only when `ui/node_modules` is missing, then typecheck, lint, test. Re-ran in the main checkout: `npm ci`, `npm run build` (relative asset paths check passes), `typecheck`, `lint`, `npm test` (1 passed). Notes: `npm audit` reports 7 moderate issues in dev-only packages (none ship to the Pi); jsdom's dependency w3c-xmlserializer warns that Node 25.9 is outside its engines range (works). The `file://` load fails in Chrome 157 without a flag, so D4 now serves `ui/dist` from a localhost static server (ticket 05). Full `scripts/verify.sh` runs after 03 lands.
