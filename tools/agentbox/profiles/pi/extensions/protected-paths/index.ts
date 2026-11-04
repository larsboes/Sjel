/**
 * Protected paths — the seatbelt, not the boundary.
 *
 * The container is what actually contains this agent: only /workspace is mounted, so
 * there is no host home to protect. This extension exists for the two cases the
 * container doesn't cover — the day someone widens a mount, and the agent's own config
 * directory, which holds the model API key in plaintext because the agent needs to
 * read it.
 *
 * Adapted from pi's own examples/extensions/protected-paths.ts (MIT, earendil-works/pi),
 * extended with read coverage, bash-command scanning and secret filename patterns.
 * Paths below are container paths, deliberately: os.homedir() inside the box is
 * /home/agent and says nothing about the host.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

const PROTECTED_DIRS = [
	"/home/agent", // the agent's own config: models.json holds the model API key
	"/opt/agent", // the agent binary itself
	"/etc",
	"/run/secrets",
	"/proc",
];

const PROTECTED_PATTERNS = [
	/\.env(\.|$)/,
	/credentials\.json$/,
	/id_rsa(\.|$)/,
	/id_ed25519(\.|$)/,
	/\.pem$/,
	/\.p12$/,
];

function offence(path: string): string | undefined {
	const dir = PROTECTED_DIRS.find((d) => path === d || path.startsWith(`${d}/`));
	if (dir) return `${dir} is off limits inside the box`;
	const pattern = PROTECTED_PATTERNS.find((p) => p.test(path));
	if (pattern) return `${path} looks like a secret (${pattern.source})`;
	return undefined;
}

/** Bash is the hole every path check leaks through: `cat /home/agent/config/models.json`
 * is a read the read-tool guard never sees. Match the same rules against the raw
 * command string — crude, and crude is the right trade here. */
function offenceInCommand(command: string): string | undefined {
	for (const dir of PROTECTED_DIRS) {
		if (command.includes(dir)) return `${dir} is off limits inside the box`;
	}
	const pattern = PROTECTED_PATTERNS.find((p) => p.test(command));
	if (pattern) return `command touches a secret-shaped path (${pattern.source})`;
	return undefined;
}

export default function (pi: ExtensionAPI) {
	pi.on("tool_call", async (event, ctx) => {
		let reason: string | undefined;

		if (event.toolName === "bash") {
			reason = offenceInCommand((event.input.command as string) ?? "");
		} else if (["read", "write", "edit", "ls", "grep", "find"].includes(event.toolName)) {
			reason = offence((event.input.path as string) ?? "");
		}

		if (!reason) return undefined;

		if (ctx.hasUI) ctx.ui.notify(`agentbox blocked ${event.toolName}: ${reason}`, "warning");
		return { block: true, reason: `Blocked by agentbox: ${reason}` };
	});
}
