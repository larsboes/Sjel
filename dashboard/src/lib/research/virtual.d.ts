// Emitted by vite/research.ts at build time.
declare module "virtual:sjel-research" {
  export const entries: { file: string; markdown: string }[];
  export const registers: {
    upstreams: import("./content").UpstreamRow[];
    systems: import("./content").SystemRow[];
  };
  export const benchmarks: import("./content").BenchmarkRun[];
}
