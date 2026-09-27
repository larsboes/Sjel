// A setting under its Sjel name, falling back to the Axon name it had before the rename on
// 2026-09-26. Same rule as libs/sjel-config/src/env.rs and tools/lib/env-compat.sh.

/** `process.env[name]` for a `SJEL_` name; the `AXON_` name is read when the new one is unset. */
export function env(name: string): string | undefined {
  const value = process.env[name];
  if (value !== undefined) return value;
  return name.startsWith("SJEL_") ? process.env[`AXON_${name.slice("SJEL_".length)}`] : undefined;
}
