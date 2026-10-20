import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

const templateRoot = join(import.meta.dir, "templates", "codex");
const baseline = Bun.TOML.parse(
  readFileSync(join(templateRoot, "config.base.toml"), "utf8"),
) as {
  default_permissions: string;
  approval_policy: { granular: Record<string, boolean> };
  approvals_reviewer: string;
  shell_environment_policy: {
    ignore_default_excludes: boolean;
    filters: Record<string, string>;
  };
  permissions: Record<string, {
    filesystem: Record<string, string | Record<string, string>>;
    network: { enabled: boolean };
  }>;
};

describe("Codex auto permissions baseline", () => {
  test("does not inherit common credential environment variables", () => {
    expect(baseline.shell_environment_policy.ignore_default_excludes).toBe(false);
    for (const pattern of ["*PASSWORD*", "*PASSWD*", "*CREDENTIAL*", "BW_SESSION"]) {
      expect(baseline.shell_environment_policy.filters[pattern]).toBe("exclude");
    }
  });

  test("allows broad ordinary command access without disabling the sandbox", () => {
    const profile = baseline.permissions[baseline.default_permissions];
    expect(profile.filesystem["/"]).toBe("write");
    expect(profile.network.enabled).toBe(true);
    expect(baseline.default_permissions).not.toBe(":danger-full-access");
  });

  test("keeps rule approval human and refuses secret-denial escalation", () => {
    expect(baseline.approval_policy.granular.rules).toBe(true);
    expect(baseline.approval_policy.granular.sandbox_approval).toBe(false);
    expect(baseline.approval_policy.granular.request_permissions).toBe(false);
    expect(baseline.approvals_reviewer).toBe("user");
  });

  test("denies credential stores and workspace private files", () => {
    const filesystem = baseline.permissions[baseline.default_permissions].filesystem;
    for (const path of [".ssh", ".aws", ".kube", ".codex/auth.json", ".pi/agent/auth.json"]) {
      expect(filesystem[`@HOME@/${path}`]).toBe("deny");
    }
    const workspace = filesystem[":workspace_roots"] as Record<string, string>;
    for (const pattern of ["**/.env", "**/*.env", "**/.secrets", "**/*.pem", "**/*.key"]) {
      expect(workspace[pattern]).toBe("deny");
    }
  });
});

const codex = Bun.which("codex");
const cases: [string[], string | undefined][] = [
  [["rm", "file.txt"], "prompt"],
  [["rm", "-rf", "build"], "prompt"],
  [["rm", "-fr", "build"], "prompt"],
  [["rm", "--recursive", "--force", "build"], "prompt"],
  [["/bin/rm", "-rf", "build"], "prompt"],
  [["/usr/bin/rm", "file.txt"], "prompt"],
  [["sudo", "rm", "-rf", "build"], "prompt"],
  [["env", "/bin/rm", "file.txt"], "prompt"],
  [["command", "rm", "file.txt"], "prompt"],
  [["cargo", "test"], undefined],
  [["git", "status"], undefined],
  [["rmdir", "empty-directory"], undefined],
];

describe("Codex deletion prefix rules (requires Codex CLI)", () => {
  for (const [command, expected] of cases) {
    test.skipIf(!codex)(command.join(" "), () => {
      const result = Bun.spawnSync([
        codex!, "execpolicy", "check", "--rules", join(templateRoot, "security.rules"),
        "--", ...command,
      ]);
      expect(result.exitCode).toBe(0);
      expect(JSON.parse(result.stdout.toString()).decision).toBe(expected);
    });
  }
});
