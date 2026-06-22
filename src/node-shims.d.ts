// Minimal ambient declarations for the few Node built-ins used by file-reading
// unit tests (e.g. the motion CSS-presence test). The project does not depend on
// @types/node; these cover only what the tests import, keeping `tsc` clean.

declare module "node:fs" {
  export function readFileSync(path: string, encoding: "utf8"): string;
}

declare module "node:path" {
  export function resolve(...segments: string[]): string;
  export function dirname(path: string): string;
}

declare module "node:url" {
  export function fileURLToPath(url: string | URL): string;
}
