// A setting under its Sjel name. The Axon name it had before the rename on 2026-09-26 was read
// as a fallback until that compatibility layer was retired on 2026-09-30, so a setting present
// only under the old name is now absent. Same rule as libs/sjel-config/src/env.rs.

/** `process.env[name]` for a setting in the environment. */
export function env(name: string): string | undefined {
  return process.env[name];
}
