/**
 * Command policy classifier — docs/plans/agent.md §3, for pi.
 *
 * Pure: no pi import, no I/O except resolving symlinks of paths that exist. Every bash command
 * and every write gets one class:
 *
 *   block  refused; the user may allow one call from a card naming the rule, never a session grant
 *   ask    approval card; a session grant is keyed by tool and exact arguments
 *   allow  runs without a card
 *
 * The steps follow the spec: split at shell operators, unwrap wrappers (`env`, `command`,
 * `timeout`, `bash -c`, `xargs`, `find -exec`, an absolute path to a binary), treat a program
 * name that comes from a substitution as opaque, resolve write targets, take the strictest class
 * of the parts, and allow every program without a rule (R4). Cards are kept for harmful classes.
 *
 * This is not a sandbox. Shell indirection is unbounded (a script can do anything its author
 * wrote into it), so the classifier narrows what an agent does by reflex; it does not contain a
 * determined one.
 */
import { existsSync, realpathSync } from "node:fs";
import { basename, dirname, isAbsolute, resolve } from "node:path";

export type PolicyClass = "allow" | "ask" | "block";

export interface Verdict {
  class: PolicyClass;
  /** Rule ID from the spec table (B1–B7, A1–A6, R1–R4). */
  rule: string;
  /** One line a human reads on the approval card. */
  why: string;
  /** Program names seen, for the log. Never arguments: they can carry secrets. */
  programs: string[];
}

export interface Scope {
  cwd: string;
  home: string;
  /** Directories a write may target besides cwd, e.g. the temp directory. */
  writable: string[];
}

const RANK: Record<PolicyClass, number> = { allow: 0, ask: 1, block: 2 };

function verdict(cls: PolicyClass, rule: string, why: string, programs: string[] = []): Verdict {
  return { class: cls, rule, why, programs };
}

export function strictest(verdicts: Verdict[]): Verdict {
  let worst = verdicts[0] ?? verdict("allow", "R1", "reads only");
  const programs = new Set<string>();
  for (const v of verdicts) {
    for (const p of v.programs) programs.add(p);
    if (RANK[v.class] > RANK[worst.class]) worst = v;
  }
  return { ...worst, programs: [...programs] };
}

// ---- lexer -------------------------------------------------------------------------------

interface Word {
  kind: "word";
  text: string;
  /** Contents of every $(…) and `…` inside the word. */
  substitutions: string[];
  /** The word contains $VAR / ${…} / a substitution: its value is unknown before it runs. */
  dynamic: boolean;
}
interface Op {
  kind: "op";
  op: string;
  /** For a redirect: the target word. For a heredoc: its body. For <( ) / >( ): the contents. */
  arg?: Word;
  body?: string;
}
type Token = Word | Op;

const REDIRECT = /^(\d*)(>>|>\||>&|&>>|&>|>|<<<|<<-|<<|<&|<>|<)/;

function readBalanced(s: string, i: number, open: string, close: string): [string, number] {
  // s[i] is just past the opening char. Returns contents and index past the closing char.
  let depth = 1;
  let j = i;
  let quote: string | null = null;
  while (j < s.length) {
    const c = s[j];
    if (quote) {
      if (c === "\\" && quote === '"') j++;
      else if (c === quote) quote = null;
    } else if (c === "'" || c === '"') quote = c;
    else if (c === "\\") j++;
    else if (c === open) depth++;
    else if (c === close && --depth === 0) return [s.slice(i, j), j + 1];
    j++;
  }
  return [s.slice(i), s.length];
}

export function lex(src: string): Token[] {
  const tokens: Token[] = [];
  const pendingHeredocs: { delimiter: string; strip: boolean; op: Op }[] = [];
  let i = 0;
  let word: Word | null = null;
  const flush = () => {
    if (word) tokens.push(word);
    word = null;
  };
  const cur = (): Word => (word ??= { kind: "word", text: "", substitutions: [], dynamic: false });

  while (i < src.length) {
    const c = src[i];
    if (c === "\n") {
      flush();
      tokens.push({ kind: "op", op: ";" });
      i++;
      // Heredoc bodies start on the line after their operator.
      while (pendingHeredocs.length) {
        const h = pendingHeredocs.shift()!;
        const lines: string[] = [];
        while (i < src.length) {
          const end = src.indexOf("\n", i);
          const line = src.slice(i, end === -1 ? src.length : end);
          i = end === -1 ? src.length : end + 1;
          if ((h.strip ? line.replace(/^\t+/, "") : line) === h.delimiter) break;
          lines.push(line);
        }
        h.op.body = lines.join("\n");
      }
      continue;
    }
    if (c === " " || c === "\t") {
      flush();
      i++;
      continue;
    }
    if (c === "#" && !word) {
      while (i < src.length && src[i] !== "\n") i++;
      continue;
    }
    if (c === "\\") {
      if (src[i + 1] === "\n") i += 2;
      else {
        cur().text += src[i + 1] ?? "";
        i += 2;
      }
      continue;
    }
    if (c === "'") {
      const end = src.indexOf("'", i + 1);
      cur().text += src.slice(i + 1, end === -1 ? src.length : end);
      i = end === -1 ? src.length : end + 1;
      continue;
    }
    if (c === '"') {
      const w = cur();
      i++;
      while (i < src.length && src[i] !== '"') {
        if (src[i] === "\\" && i + 1 < src.length) {
          w.text += src[i + 1];
          i += 2;
        } else if (src[i] === "$" && src[i + 1] === "(") {
          const [inner, next] = readBalanced(src, i + 2, "(", ")");
          w.substitutions.push(inner);
          w.dynamic = true;
          w.text += `$(${inner})`;
          i = next;
        } else if (src[i] === "`") {
          const end = src.indexOf("`", i + 1);
          const inner = src.slice(i + 1, end === -1 ? src.length : end);
          w.substitutions.push(inner);
          w.dynamic = true;
          w.text += `\`${inner}\``;
          i = end === -1 ? src.length : end + 1;
        } else {
          if (src[i] === "$" && /[A-Za-z_{@*#?$!0-9]/.test(src[i + 1] ?? "")) w.dynamic = true;
          w.text += src[i];
          i++;
        }
      }
      i++;
      continue;
    }
    if (c === "$" && src[i + 1] === "(") {
      const [inner, next] = readBalanced(src, i + 2, "(", ")");
      const w = cur();
      w.substitutions.push(inner);
      w.dynamic = true;
      w.text += `$(${inner})`;
      i = next;
      continue;
    }
    if (c === "`") {
      const end = src.indexOf("`", i + 1);
      const inner = src.slice(i + 1, end === -1 ? src.length : end);
      const w = cur();
      w.substitutions.push(inner);
      w.dynamic = true;
      w.text += `\`${inner}\``;
      i = end === -1 ? src.length : end + 1;
      continue;
    }
    if (c === "$" && /[A-Za-z_{@*#?$!0-9]/.test(src[i + 1] ?? "")) {
      cur().dynamic = true;
      cur().text += c;
      i++;
      continue;
    }
    if ((c === "<" || c === ">") && src[i + 1] === "(" ) {
      flush();
      const [inner, next] = readBalanced(src, i + 2, "(", ")");
      tokens.push({ kind: "op", op: `${c}(`, body: inner });
      i = next;
      continue;
    }
    // Redirects, including an fd prefix that is the whole current word (2>, 1>&2).
    const prefixFd = word && /^\d+$/.test((word as Word).text) && !(word as Word).dynamic ? (word as Word).text : "";
    const m = REDIRECT.exec(prefixFd + src.slice(i));
    if ((c === "<" || c === ">" || (c === "&" && src[i + 1] === ">")) && m) {
      if (prefixFd) word = null;
      flush();
      const op = m[2];
      i += m[0].length - prefixFd.length;
      while (src[i] === " " || src[i] === "\t") i++;
      // Read the target as one word through the same lexer rules.
      const rest = lex(src.slice(i).split(/[\s;&|()<>]/, 1)[0] ?? "");
      const target = (rest.find((t) => t.kind === "word") as Word | undefined) ?? {
        kind: "word", text: "", substitutions: [], dynamic: false,
      };
      // Advance past the raw target text.
      let j = i;
      let q: string | null = null;
      while (j < src.length) {
        const ch = src[j];
        if (q) { if (ch === q) q = null; }
        else if (ch === "'" || ch === '"') q = ch;
        else if (/[\s;&|()<>]/.test(ch)) break;
        j++;
      }
      i = j;
      const tok: Op = { kind: "op", op, arg: target };
      tokens.push(tok);
      if (op === "<<" || op === "<<-") {
        pendingHeredocs.push({ delimiter: target.text, strip: op === "<<-", op: tok });
      }
      continue;
    }
    const two = src.slice(i, i + 2);
    if (two === "&&" || two === "||" || two === "|&" || two === ";;") {
      flush();
      tokens.push({ kind: "op", op: two === "|&" ? "|" : two === ";;" ? ";" : two });
      i += 2;
      continue;
    }
    if (c === ";" || c === "|" || c === "&" || c === "(" || c === ")") {
      flush();
      tokens.push({ kind: "op", op: c });
      i++;
      continue;
    }
    cur().text += c;
    i++;
  }
  flush();
  return tokens;
}

// ---- parse into simple commands ----------------------------------------------------------

interface Simple {
  words: Word[];
  redirects: Op[];
  /** stdin comes from a pipe. */
  piped: boolean;
}

function parse(tokens: Token[]): Simple[] {
  const out: Simple[] = [];
  let current: Simple = { words: [], redirects: [], piped: false };
  const push = (pipedNext: boolean) => {
    if (current.words.length || current.redirects.length) out.push(current);
    current = { words: [], redirects: [], piped: pipedNext };
  };
  for (const t of tokens) {
    if (t.kind === "word") {
      // Grouping and control words split commands; the commands inside are classified alone.
      if (current.words.length === 0 && ["{", "}", "!", "then", "do", "else", "elif", "fi", "done", "if", "while", "until", "esac"].includes(t.text)) {
        continue;
      }
      current.words.push(t);
    } else if (t.op === "|") push(true);
    else if ([";", "&&", "||", "&", "(", ")"].includes(t.op)) push(false);
    else current.redirects.push(t);
  }
  push(false);
  return out;
}

// ---- rules --------------------------------------------------------------------------------

const BLOCK_PROGRAMS: Record<string, string> = {
  rm: "B1", shred: "B5", dd: "B5", fdisk: "B5", sfdisk: "B5", parted: "B5", wipefs: "B5",
  diskutil: "B5", csrutil: "B5", spctl: "B5", nvram: "B5", launchctl: "B5", systemctl: "B5",
  sudo: "B3", su: "B3", doas: "B3", pkexec: "B3",
};
const INTERPRETERS = new Set(["sh", "bash", "zsh", "dash", "ksh", "fish", "python", "python3", "node", "ruby", "perl"]);
const SHELLS = new Set(["sh", "bash", "zsh", "dash", "ksh"]);

const R1 = new Set([
  "ls", "cat", "head", "tail", "wc", "rg", "grep", "egrep", "fgrep", "fd", "tree", "stat", "file", "du",
  "df", "pwd", "echo", "printf", "which", "type", "date", "sort", "uniq", "cut", "tr", "nl", "column",
  "diff", "cmp", "comm", "md5sum", "sha1sum", "sha256sum", "sha512sum", "cksum", "basename", "dirname",
  "realpath", "readlink", "jq", "true", "false", "test", "[", "cd", "pushd", "popd", "uname", "id",
  "whoami", "hostname", "ps", "uptime", "free", "nproc", "sleep", "seq", "xxd", "hexdump", "od",
  "strings", "bat", "eza", "man", "export", "set", "unset", "local", "read", "wait", "exit", "return",
  "shift", "trap", ":", "for", "case", "in",
]);
const GIT_READS = new Set([
  "status", "diff", "log", "show", "blame", "rev-parse", "ls-files", "grep", "shortlog", "describe",
  "cat-file", "ls-tree", "rev-list", "merge-base", "name-rev", "for-each-ref", "show-ref", "check-ignore",
  "whatchanged", "help", "version",
]);
const GIT_LOCAL = new Set(["fetch", "add", "rm", "mv", "clone", "init"]);
const GIT_ASK = new Set([
  "commit", "push", "merge", "rebase", "cherry-pick", "stash", "tag", "branch", "switch", "checkout",
  "pull", "apply", "am", "revert", "reset", "restore", "submodule", "worktree", "notes", "config",
  "remote", "gc", "prune", "bisect", "update-index",
]);

/** Package managers and the subcommands that change packages (A3). `"*"`: every subcommand. */
const PACKAGE_CHANGES: Record<string, Set<string>> = {
  npm: new Set(["install", "i", "ci", "add", "remove", "rm", "uninstall", "un", "update", "up", "upgrade", "link", "exec", "x"]),
  pnpm: new Set(["install", "i", "add", "remove", "rm", "uninstall", "update", "up", "upgrade", "link", "dlx", "exec"]),
  yarn: new Set(["install", "add", "remove", "upgrade", "up", "link", "dlx"]),
  bun: new Set(["install", "i", "add", "a", "remove", "rm", "update", "upgrade", "link", "x"]),
  brew: new Set(["install", "reinstall", "upgrade", "uninstall", "remove", "rm", "tap", "untap", "link", "unlink", "cleanup", "update"]),
  pip: new Set(["install", "uninstall", "download"]),
  pip3: new Set(["install", "uninstall", "download"]),
  uv: new Set(["add", "remove", "sync", "pip", "tool", "lock", "self"]),
  cargo: new Set(["install", "uninstall", "add", "remove", "update"]),
  rustup: new Set(["install", "uninstall", "update", "default", "toolchain", "target", "component", "override", "self", "set"]),
  gem: new Set(["install", "uninstall", "update"]),
  go: new Set(["install", "get"]),
  npx: new Set(["*"]), bunx: new Set(["*"]), uvx: new Set(["*"]), pipx: new Set(["*"]),
  apt: new Set(["*"]), "apt-get": new Set(["*"]), dpkg: new Set(["-i", "--install", "-r", "--remove", "-P", "--purge"]),
  dnf: new Set(["*"]), yum: new Set(["*"]), snap: new Set(["*"]), flatpak: new Set(["*"]),
};
const CURL_SENDS = new Set([
  "-d", "-F", "-T", "--form", "--form-string", "--json", "--upload-file", "--post-data", "--post-file",
  "--body-data", "--body-file",
]);
const REMOTE_SHELLS = new Set(["ssh", "scp", "sftp", "nc", "ncat", "telnet", "ftp"]);
const PROCESSES = new Set(["kill", "pkill", "killall", "docker", "podman", "orb"]);
const DOCKER_READS = new Set(["ps", "images", "logs", "inspect", "version", "info", "stats", "top"]);
const WRITERS = new Set(["cp", "mv", "mkdir", "touch", "ln", "tee", "install", "truncate", "rmdir", "unlink", "chmod", "chown", "chgrp"]);

const NO_WRITE_TARGETS = new Set(["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/tty"]);

function canonical(path: string): string {
  // Follow symlinks for the part that exists, keep the rest lexical.
  let existing = path;
  const missing: string[] = [];
  while (!existsSync(existing)) {
    const parent = dirname(existing);
    if (parent === existing) return path;
    missing.unshift(basename(existing));
    existing = parent;
  }
  try {
    return resolve(realpathSync(existing), ...missing);
  } catch {
    return path;
  }
}

export function resolvePath(raw: string, scope: Scope): string {
  let p = raw;
  if (p === "~" || p.startsWith("~/")) p = scope.home + p.slice(1);
  return canonical(isAbsolute(p) ? resolve(p) : resolve(scope.cwd, p));
}

function within(path: string, root: string): boolean {
  const r = canonical(resolve(root));
  return path === r || path.startsWith(r.endsWith("/") ? r : `${r}/`);
}

export function inScope(path: string, scope: Scope): boolean {
  return [scope.cwd, ...scope.writable].some((root) => within(path, root));
}

/** Whether `path` is under the home directory but outside the scope: the one write that asks (A1). */
function homeOutsideScope(path: string, scope: Scope): boolean {
  return !inScope(path, scope) && within(path, scope.home);
}

/** A write to `raw`: Ask under $HOME outside the scope, otherwise Allow (spec step 4). */
function writeTarget(raw: Word, scope: Scope, program: string): Verdict | null {
  if (raw.dynamic) return verdict("ask", "A1", `${program} writes to a path only known when it runs`, [program]);
  if (NO_WRITE_TARGETS.has(raw.text)) return null;
  const path = resolvePath(raw.text.replace(/[*?[].*$/, "") || ".", scope);
  if (homeOutsideScope(path, scope)) {
    return verdict("ask", "A1", `${program} writes under the home directory outside the working directory (${raw.text})`, [program]);
  }
  return verdict("allow", "R4", `${program} writes ${raw.text}`, [program]);
}

function shortFlags(args: string[]): Set<string> {
  const flags = new Set<string>();
  for (const a of args) {
    if (a === "--") break;
    if (a.startsWith("--")) flags.add(a);
    else if (a.startsWith("-") && a.length > 1) for (const ch of a.slice(1)) flags.add(`-${ch}`);
  }
  return flags;
}

function classifyRm(args: Word[], scope: Scope, unknownArgs: boolean): Verdict {
  const p = ["rm"];
  if (unknownArgs) return verdict("block", "B1", "rm with arguments chosen at run time", p);
  const flags = shortFlags(args.map((a) => a.text));
  if (["-r", "-R", "-f", "--recursive", "--force"].some((f) => flags.has(f))) {
    return verdict("block", "B1", "recursive or forced rm", p);
  }
  const targets = args.filter((a) => !a.text.startsWith("-"));
  for (const t of targets) {
    if (t.dynamic) return verdict("ask", "A6", "rm of a path only known when it runs", p);
    const path = resolvePath(t.text.replace(/[*?[].*$/, "") || ".", scope);
    if (path === "/" || path === canonical(scope.home) || path === canonical(scope.cwd) || !inScope(path, scope)) {
      return verdict("block", "B1", `rm of ${t.text}`, p);
    }
  }
  return verdict("allow", "R4", "rm of a file in the working directory", p);
}

function classifyGit(args: Word[]): Verdict {
  const p = ["git"];
  let i = 0;
  // Global options before the subcommand.
  while (i < args.length && args[i].text.startsWith("-")) {
    if (["-C", "-c", "--git-dir", "--work-tree", "--namespace"].includes(args[i].text)) i += 2;
    else i += 1;
  }
  const sub = args[i]?.text ?? "";
  const rest = args.slice(i + 1).map((a) => a.text);
  const flags = shortFlags(rest);
  const has = (...f: string[]) => f.some((x) => flags.has(x) || rest.includes(x));
  if (!sub) return verdict("allow", "R2", "git without a subcommand", p);
  if (sub === "reset" && has("--hard", "--merge", "--keep")) return verdict("block", "B2", "git reset discards work", p);
  if (sub === "clean" && has("-f", "--force")) return verdict("block", "B2", "git clean deletes untracked files", p);
  if (sub === "checkout" && (rest.includes("--") || rest.includes(".") || has("-f", "--force"))) {
    return verdict("block", "B2", "git checkout overwrites worktree files", p);
  }
  if (sub === "restore" && !(has("--staged", "-S") && !has("--worktree", "-W"))) {
    return verdict("block", "B2", "git restore overwrites worktree files", p);
  }
  if (sub === "push" && (has("-f", "--force", "--force-with-lease", "--force-if-includes", "--mirror", "-d", "--delete", "--prune") ||
      rest.some((r) => r.startsWith("+") || (r.startsWith(":") && r.length > 1) || r.startsWith("--force")))) {
    return verdict("block", "B2", "git push rewrites or deletes remote history", p);
  }
  if (sub === "branch" && (has("-D") || (has("-d", "--delete") && has("-f", "--force")) || has("-M", "-C"))) {
    return verdict("block", "B2", "git branch force-deletes or overwrites a branch", p);
  }
  if (sub === "tag" && has("-d", "--delete", "-f", "--force")) return verdict("block", "B2", "git tag deletes or moves a tag", p);
  if (sub === "filter-repo" || sub === "filter-branch") return verdict("block", "B2", `git ${sub} rewrites history`, p);
  if (sub === "reflog" && (rest[0] === "expire" || rest[0] === "delete")) return verdict("block", "B2", "git reflog loses history", p);
  if (sub === "stash" && (rest[0] === "drop" || rest[0] === "clear")) return verdict("block", "B2", "git stash drops saved work", p);
  if (sub === "update-ref" && has("-d")) return verdict("block", "B2", "git update-ref deletes a ref", p);
  if (GIT_READS.has(sub)) return verdict("allow", "R2", `git ${sub}`, p);
  if (sub === "branch" && (rest.length === 0 || rest.every((r) => ["-a", "-r", "-l", "--list", "-v", "-vv", "--show-current", "--all", "--remotes"].includes(r) || r.startsWith("--format") || r.startsWith("--sort") || r.startsWith("--contains") || r.startsWith("--merged")))) {
    return verdict("allow", "R2", "git branch (list)", p);
  }
  if (sub === "tag" && (rest.length === 0 || has("-l", "--list"))) return verdict("allow", "R2", "git tag (list)", p);
  if (sub === "remote" && (rest.length === 0 || rest.every((r) => r === "-v" || r === "show" || r === "get-url"))) {
    return verdict("allow", "R2", "git remote (list)", p);
  }
  if (sub === "config" && has("--get", "--get-all", "--list", "-l", "--get-regexp")) return verdict("allow", "R2", "git config (read)", p);
  if (sub === "stash" && (rest[0] === "list" || rest[0] === "show")) return verdict("allow", "R2", "git stash (read)", p);
  if (sub === "reflog") return verdict("allow", "R2", "git reflog (read)", p);
  if (sub === "restore" && has("--staged", "-S")) return verdict("allow", "R2", "git restore --staged", p);
  if (GIT_LOCAL.has(sub)) return verdict("allow", "R2", `git ${sub}`, p);
  if (GIT_ASK.has(sub)) return verdict("ask", "A2", `git ${sub} changes history, branches or remotes`, p);
  return verdict("allow", "R4", `git ${sub}`, p);
}

function classifyGh(args: string[]): Verdict {
  const p = ["gh"];
  const [area, action] = args;
  if (area === "release" && action === "create") return verdict("block", "B7", "gh release create publishes", p);
  if (area === "repo" && action === "delete") return verdict("block", "B7", "gh repo delete", p);
  const reads = new Set(["list", "view", "diff", "checks", "status", "watch", "download"]);
  if (area === "api") {
    const method = args.find((a, i) => args[i - 1] === "-X" || args[i - 1] === "--method");
    const writes = args.some((a) => ["-f", "-F", "--field", "--raw-field", "--input"].includes(a));
    return method && method.toUpperCase() !== "GET" || writes
      ? verdict("ask", "A4", "gh api write", p)
      : verdict("allow", "R1", "gh api read", p);
  }
  if (reads.has(action ?? "") || ["auth", "--version", "version", "help"].includes(area ?? "")) {
    return area === "auth" && action !== "status"
      ? verdict("ask", "A4", "gh auth changes credentials", p)
      : verdict("allow", "R1", `gh ${area} ${action ?? ""}`.trim(), p);
  }
  return verdict("ask", "A4", `gh ${area ?? ""} ${action ?? ""} writes to GitHub`.replace(/\s+/g, " "), p);
}

function strip(word: string): string {
  // `/bin/rm` and `/usr/bin/git` are rm and git; a relative script stays itself.
  return isAbsolute(word) ? basename(word) : word;
}

const WRAPPER_ARGS: Record<string, Set<string>> = {
  env: new Set(["-u", "--unset", "-C", "--chdir", "-S", "--split-string"]),
  nice: new Set(["-n", "--adjustment"]),
  timeout: new Set(["-s", "--signal", "-k", "--kill-after"]),
  stdbuf: new Set(["-i", "-o", "-e"]),
  xargs: new Set(["-n", "-I", "-P", "-d", "-L", "-E", "-s", "-a", "--max-args", "--max-procs", "--delimiter", "--replace", "--arg-file"]),
};

/** Classify one simple command given as words. `unknownArgs`: reached via xargs or find -exec. */
function classifyWords(words: Word[], scope: Scope, piped: boolean, unknownArgs: boolean, depth: number): Verdict {
  if (depth > 8) return verdict("ask", "A6", "nesting too deep to classify");
  let ws = words;
  // Assignments before the program: VAR=value cmd.
  while (ws.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(ws[0].text)) {
    const subs = ws[0].substitutions;
    if (subs.length) {
      const inner = strictest(subs.map((s) => classify(s, scope, depth + 1)));
      if (inner.class !== "allow") return inner;
    }
    ws = ws.slice(1);
  }
  if (!ws.length) return verdict("allow", "R1", "assignment");

  const head = ws[0];
  // Step 3: a program name that comes from a substitution or a variable.
  if (head.dynamic) {
    const inner = head.substitutions.map((s) => classify(s, scope, depth + 1));
    const reached = head.substitutions.join(" ").split(/[^A-Za-z0-9_./-]+/).map(strip)
      .find((name) => BLOCK_PROGRAMS[name]);
    if (reached) return verdict("block", BLOCK_PROGRAMS[reached], `program name built at run time reaches ${reached}`, [reached]);
    return strictest([verdict("ask", "A6", "program name built at run time"), ...inner]);
  }

  const prog = head.text === "tools/doctor" || head.text === "./tools/doctor" ? "tools/doctor" : strip(head.text);
  const args = ws.slice(1);
  const texts = args.map((a) => a.text);
  const p = [prog];

  // Substitutions in arguments run too: classify them.
  const argSubs = args.flatMap((a) => a.substitutions).map((s) => classify(s, scope, depth + 1));
  const withSubs = (v: Verdict) => strictest([v, ...argSubs]);

  // Step 2: wrappers.
  if (prog === "env" || prog === "nice" || prog === "nohup" || prog === "time" || prog === "timeout" ||
      prog === "command" || prog === "builtin" || prog === "exec" || prog === "stdbuf") {
    if (prog === "command" && (texts[0] === "-v" || texts[0] === "-V")) {
      return verdict("allow", "R1", "command lookup", p);
    }
    let j = 0;
    const takesArg = WRAPPER_ARGS[prog] ?? new Set<string>();
    while (j < args.length) {
      const t = args[j].text;
      if (prog === "env" && /^[A-Za-z_][A-Za-z0-9_]*=/.test(t)) { j++; continue; }
      if (prog === "env" && (t === "-S" || t === "--split-string") && args[j + 1]) {
        return withSubs(classify(args[j + 1].text, scope, depth + 1));
      }
      if (t.startsWith("-")) { j += takesArg.has(t) ? 2 : 1; continue; }
      break;
    }
    if (prog === "timeout") j++; // the duration
    if (j >= args.length) return verdict("allow", "R1", `${prog} without a command`, p);
    return withSubs(classifyWords(args.slice(j), scope, piped, unknownArgs, depth + 1));
  }
  if (prog === "busybox" && args.length) return classifyWords(args, scope, piped, unknownArgs, depth + 1);
  if (prog === "xargs") {
    let j = 0;
    while (j < args.length && args[j].text.startsWith("-")) j += WRAPPER_ARGS.xargs.has(args[j].text) ? 2 : 1;
    if (j >= args.length) return verdict("allow", "R1", "xargs echo", p);
    return classifyWords(args.slice(j), scope, piped, true, depth + 1);
  }
  if (prog === "find") {
    if (texts.includes("-delete")) return verdict("block", "B1", "find -delete", p);
    const exec = texts.findIndex((t) => ["-exec", "-execdir", "-ok", "-okdir"].includes(t));
    if (exec !== -1) {
      const end = texts.findIndex((t, k) => k > exec && (t === ";" || t === "+" || t === "\\;"));
      const cmd = args.slice(exec + 1, end === -1 ? undefined : end).filter((w) => w.text !== "{}");
      return cmd.length ? classifyWords(cmd, scope, false, true, depth + 1) : verdict("ask", "A6", "find -exec", p);
    }
    const writes = texts.findIndex((t) => ["-fprint", "-fprint0", "-fprintf", "-fls"].includes(t));
    if (writes !== -1 && args[writes + 1]) return writeTarget(args[writes + 1], scope, "find") ?? verdict("allow", "R1", "find", p);
    return verdict("allow", "R1", "find", p);
  }
  if (SHELLS.has(prog)) {
    const c = texts.findIndex((t) => t === "-c" || /^-[a-z]*c$/.test(t));
    if (c !== -1 && args[c + 1]) {
      if (args[c + 1].dynamic) return verdict("ask", "A6", `${prog} -c with a script built at run time`, p);
      return withSubs(classify(args[c + 1].text, scope, depth + 1));
    }
  }
  if (INTERPRETERS.has(prog)) {
    const hasScript = texts.some((t) => !t.startsWith("-") || t === "-c" || t === "-e" || t === "-m" || t === "--eval");
    if (piped && !hasScript) return verdict("block", "B4", `piped input executed by ${prog}`, p);
    return withSubs(verdict("allow", "R4", `${prog} runs code`, p));
  }
  if (prog === "eval") return withSubs(strictest([verdict("ask", "A6", "eval", p), classify(texts.join(" "), scope, depth + 1)]));
  if (prog === "source" || prog === ".") return withSubs(verdict("allow", "R4", "source runs a file", p));

  // Step 4–6: rules.
  if (BLOCK_PROGRAMS[prog] === "B3") return verdict("block", "B3", `${prog} gains privileges`, p);
  if (prog === "osascript" && texts.join(" ").includes("administrator privileges")) {
    return verdict("block", "B3", "osascript with administrator privileges", p);
  }
  if (prog === "rm") return withSubs(classifyRm(args, scope, unknownArgs));
  if (prog === "git") {
    // Via xargs or find -exec, the visible arguments still count: a visible --force blocks.
    const git = classifyGit(args);
    return withSubs(unknownArgs ? strictest([verdict("ask", "A6", "git with arguments chosen at run time", p), git]) : git);
  }
  if (prog === "gh") return withSubs(classifyGh(texts));
  if (BLOCK_PROGRAMS[prog] === "B5" || /^mkfs(\.|$)/.test(prog)) return verdict("block", "B5", `${prog} changes disks or system state`, p);
  if (prog === "crontab" && texts.includes("-r")) return verdict("block", "B5", "crontab -r", p);
  if (prog === "defaults" && texts[0] === "write") return verdict("block", "B5", "defaults write", p);
  if ((prog === "chmod" || prog === "chown" || prog === "chgrp") && shortFlags(texts).has("-R")) {
    return verdict("block", "B5", `recursive ${prog}`, p);
  }
  if (prog === "security" && (/^find-.*-password$/.test(texts[0] ?? "") && texts.includes("-w") || /^delete-/.test(texts[0] ?? ""))) {
    return verdict("block", "B6", "keychain secret", p);
  }
  if (prog === "secret-tool" && ["lookup", "clear"].includes(texts[0] ?? "")) return verdict("block", "B6", "keyring secret", p);
  if ((prog === "cargo" || prog === "npm" || prog === "bun" || prog === "pnpm" || prog === "yarn" || prog === "uv") && texts[0] === "publish") {
    return verdict("block", "B7", `${prog} publish`, p);
  }
  if (prog === "twine" && texts[0] === "upload") return verdict("block", "B7", "twine upload", p);

  const changes = PACKAGE_CHANGES[prog];
  if (changes) {
    const sub = texts.find((t) => !t.startsWith("-") || changes.has(t)) ?? "";
    if (changes.has("*") || changes.has(sub)) return verdict("ask", "A3", `${prog} ${sub} changes packages or runs downloaded code`.replace(/\s+/g, " "), p);
    // A bare `bun`/`npm`/`pnpm`/`yarn` with no subcommand installs.
    if (!sub && ["bun", "npm", "pnpm", "yarn"].includes(prog)) return verdict("ask", "A3", `${prog} installs`, p);
    return withSubs(verdict("allow", "R3", `${prog} ${sub}`.trim(), p));
  }
  if (prog === "tools/doctor") return verdict("allow", "R3", "tools/doctor", p);
  if (prog === "sjel" && ["help", "search", "context", "status", "--help", undefined].includes(texts[0])) {
    return verdict("allow", "R3", `sjel ${texts[0] ?? ""}`.trim(), p);
  }
  if (prog === "curl" || prog === "wget") {
    const parts: Verdict[] = [];
    const out = args.findIndex((a) => ["-o", "--output", "-O", "--output-document"].includes(a.text));
    if (out !== -1 && args[out + 1] && !(prog === "curl" && args[out].text === "-O")) {
      const w = writeTarget(args[out + 1], scope, prog);
      if (w) parts.push(w);
    }
    const sends = texts.some((t, i) => {
      const flag = t.split("=")[0];
      if (CURL_SENDS.has(flag) || flag.startsWith("--data") || /^-[dF]./.test(t)) return true;
      if (t === "-X" || t === "--request" || t === "--method") return !/^(GET|HEAD)$/i.test(texts[i + 1] ?? "");
      const method = /^(?:-X|--request=|--method=)(.+)$/.exec(t);
      return method ? !/^(GET|HEAD)$/i.test(method[1]) : false;
    });
    parts.push(sends ? verdict("ask", "A4", `${prog} sends data`, p) : verdict("allow", "R1", `${prog} reads`, p));
    return withSubs(strictest(parts));
  }
  if (REMOTE_SHELLS.has(prog)) return withSubs(verdict("ask", "A4", `${prog} reaches a remote host`, p));
  if (prog === "rsync") {
    const operands = texts.filter((t) => !t.startsWith("-"));
    if (operands.some((t) => /^[^/]*:/.test(t))) return withSubs(verdict("ask", "A4", "rsync to or from a remote", p));
    const vs = operands.slice(-1).map((t) => writeTarget(args.find((a) => a.text === t)!, scope, prog)).filter((v): v is Verdict => v !== null);
    return withSubs(strictest([verdict("allow", "R4", "rsync on this machine", p), ...vs]));
  }
  if (PROCESSES.has(prog)) {
    if ((prog === "docker" || prog === "podman") && DOCKER_READS.has(texts[0] ?? "")) {
      return verdict("allow", "R1", `${prog} ${texts[0]}`, p);
    }
    return withSubs(verdict("ask", "A5", `${prog} affects processes or containers`, p));
  }
  if (WRITERS.has(prog)) {
    const targets = args.filter((a) => !a.text.startsWith("-"));
    // cp/mv/ln/install write their last argument; tee, mkdir, touch, truncate every argument.
    const written = ["cp", "mv", "ln", "install"].includes(prog) ? targets.slice(-1) : targets;
    if (prog === "mv") written.push(...targets.slice(0, -1)); // the source disappears
    const vs = written.map((t) => writeTarget(t, scope, prog)).filter((v): v is Verdict => v !== null);
    return withSubs(strictest([verdict("allow", "R4", `${prog} writes files`, p), ...vs]));
  }
  if (R1.has(prog)) return withSubs(verdict("allow", "R1", prog, p));
  return withSubs(verdict("allow", "R4", `${prog} has no rule`, p));
}

function classifySimple(cmd: Simple, scope: Scope, depth: number): Verdict {
  const parts: Verdict[] = [classifyWords(cmd.words, scope, cmd.piped, false, depth)];
  const prog = cmd.words.length ? strip(cmd.words.find((w) => !/^[A-Za-z_][A-Za-z0-9_]*=/.test(w.text))?.text ?? "") : "";
  for (const r of cmd.redirects) {
    if (r.op === "<(" || r.op === ">(") {
      // Process substitution into an interpreter is download-and-run.
      if (INTERPRETERS.has(prog) || prog === "source" || prog === ".") {
        parts.push(verdict("block", "B4", `process substitution executed by ${prog}`, [prog]));
      }
      parts.push(classify(r.body ?? "", scope, depth + 1));
      continue;
    }
    if (r.op === "<<" || r.op === "<<-") {
      if (SHELLS.has(prog)) parts.push(classify(r.body ?? "", scope, depth + 1));
      continue;
    }
    if (r.op === "<" || r.op === "<<<" || r.op === "<&" || r.op === ">&" && /^\d*-?$/.test(r.arg?.text ?? "")) continue;
    if (r.arg && r.arg.substitutions.length) parts.push(...r.arg.substitutions.map((s) => classify(s, scope, depth + 1)));
    if (r.arg) {
      const w = writeTarget(r.arg, scope, prog || "redirect");
      if (w) parts.push(w);
    }
  }
  // A redirect that writes inside the scope turns a read into A1 (spec, "Rules").
  return strictest(parts);
}

/** Classify a shell command string. */
export function classify(command: string, scope: Scope, depth = 0): Verdict {
  if (depth > 8) return verdict("ask", "A6", "nesting too deep to classify");
  const commands = parse(lex(command));
  if (!commands.length) return verdict("allow", "R1", "empty command");
  return strictest(commands.map((c) => classifySimple(c, scope, depth)));
}

/** A write through pi's `write` or `edit` tool, judged by step 4 like a shell write. */
export function classifyWrite(path: string, scope: Scope, tool: string): Verdict {
  const resolved = resolvePath(path, scope);
  if (homeOutsideScope(resolved, scope)) {
    return verdict("ask", "A1", `${tool} under the home directory outside the working directory (${path})`, [tool]);
  }
  return verdict("allow", "R4", `${tool} ${inScope(resolved, scope) ? "inside" : "outside"} the working directory`, [tool]);
}
