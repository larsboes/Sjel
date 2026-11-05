// tools/pi-learn.test.ts — the pure parts of the `pi-learn` Obsidian plugin: the learning-path
// schema reader and the pi launch command per platform. The Obsidian view needs Obsidian and is
// checked by hand (Packs/learning/plugins/pi-learn/README.md, Check).
import { describe, expect, test } from "bun:test";
import { buildLaunch, splitModel } from "../Packs/learning/plugins/pi-learn/src/launch";
import { parsePath, progress, sittingPrompt } from "../Packs/learning/plugins/pi-learn/src/path";

const note = (graph: string) => `---
type: note
---
# X — Learning Path

## Goal

Something.

## Learning Path

\`\`\`mermaid
${graph}
\`\`\`

## Rungs

\`\`\`mermaid
graph TD
  Z[Not this one]:::held
\`\`\`
`;

describe("learning path schema", () => {
	test("states come from :::class and from class lines; the rest is open", () => {
		const path = parsePath(
			note(`graph TD
  classDef held fill:#2e7d32
  latency["Latency Hierarchy"]:::held --> vm[Virtual Memory]:::current
  vm -->|needs| threads(OS Threads)
  threads --> tokio{{Tokio Runtime}}
  class threads held`),
		)!;
		expect(path.nodes.map((n) => [n.id, n.label, n.state])).toEqual([
			["latency", "Latency Hierarchy", "held"],
			["vm", "Virtual Memory", "current"],
			["threads", "OS Threads", "held"],
			["tokio", "Tokio Runtime", "open"],
		]);
		expect(progress(path)).toEqual({ held: 2, current: 1, open: 1 });
	});

	test("an edge label is not a node", () => {
		const path = parsePath(note("graph LR\n  a --> |depends on| b"))!;
		expect(path.nodes.map((n) => n.id)).toEqual(["a", "b"]);
	});

	test("a note without the section is not a path", () => {
		expect(parsePath("# Tokio Runtime\n\n## Claims\n")).toBeNull();
	});
});

describe("pi launch", () => {
	const settings = { piCommand: "pi", launcher: "native" as const, wslDistro: "" };

	test("macOS and Linux start pi through the login shell, in the vault, with the questions protocol", () => {
		const l = buildLaunch(settings, "/v", "darwin", { SHELL: "/bin/zsh" });
		expect(l).toMatchObject({ command: "/bin/zsh", args: ["-lc", "exec pi --mode rpc"], cwd: "/v" });
		expect(l.env.PI_QUESTIONS_CLIENT).toBe("pi-questions/v1");
	});

	test("Windows native runs pi through cmd in the vault", () => {
		const l = buildLaunch(settings, "C:\\Vault", "win32", {});
		expect(l).toMatchObject({ command: "pi", args: ["--mode", "rpc"], cwd: "C:\\Vault", shell: true });
	});

	test("Windows WSL translates the vault path and sets the protocol inside WSL", () => {
		const l = buildLaunch({ ...settings, launcher: "wsl", wslDistro: "Ubuntu" }, "C:\\Users\\it's\\Vault", "win32", {});
		expect(l.command).toBe("wsl.exe");
		expect(l.args.slice(0, 5)).toEqual(["-d", "Ubuntu", "-e", "bash", "-lc"]);
		expect(l.args[5]).toBe(
			`cd "$(wslpath -u 'C:\\Users\\it'\\''s\\Vault')" && PI_QUESTIONS_CLIENT=pi-questions/v1 exec pi --mode rpc`,
		);
	});

	test("a model spec splits at the first slash only", () => {
		expect(splitModel("nvidia/deepseek-ai/deepseek-v4.1-flash")).toEqual({
			provider: "nvidia",
			modelId: "deepseek-ai/deepseek-v4.1-flash",
		});
		expect(splitModel("deepseek-flash")).toBeNull();
	});
});

describe("sitting prompt follows the open note", () => {
	test("a concept note on a path continues the path at that note", () => {
		expect(sittingPrompt("teach", "[[P]]", "[[Tokio Runtime]]", "")).toBe(
			"/skill:teach Continue the learning path [[P]] at [[Tokio Runtime]].",
		);
	});
	test("a note on no path is taught from itself", () => {
		expect(sittingPrompt("teach", null, "[[Little's Law]]", "")).toMatch(/^\/skill:teach Teach \[\[Little's Law\]\]/);
	});
	test("practice targets the note's claims, else the path's held nodes, else the topic", () => {
		expect(sittingPrompt("practice", "[[P]]", "[[N]]", "")).toBe("/skill:practice Practice the claims of [[N]], on the learning path [[P]].");
		expect(sittingPrompt("practice", "[[P]]", null, "")).toMatch(/held nodes of the learning path \[\[P\]\]/);
		expect(sittingPrompt("practice", null, null, "Kafka lag")).toBe("/skill:practice Kafka lag");
	});
});
