//! Every command and every write, in one of three classes: Block, Ask or Allow.
//!
//! This is the rule set in `docs/plans/agent.md` §3, in one place so the CLI and the capability
//! cannot disagree about what a command may do. The gate reads [`Decision::class`]: `Allow` runs
//! without a card, `Ask` shows the approval card, and `Block` shows the separate card that names
//! [`Decision::rule`] and offers no session grant.
//!
//! **This is not a sandbox.** Shell indirection is unbounded: a command can write outside the
//! scope without naming the path, and a wrapper this file has never seen runs whatever it likes.
//! The same argument `ext/guard.rs` makes about secret paths. What this buys is that the obvious
//! and the common are decided before a human is asked, so the cards that remain are worth reading.
//!
//! The order the spec fixes, and the order the code follows:
//!
//! 1. Lex the command with shell quoting rules, then split it into simple commands at `;`, `&&`,
//!    `||`, `|`, `&` and newlines. A subshell is classified by its contents.
//! 2. Unwrap what only runs another command (`env`, `command`, `nice`, `timeout`, `bash -c`,
//!    `xargs`, `find -exec`, an absolute path to a binary, a leading `\`).
//! 3. A program name that is computed (`$(…)`, backticks, `$VAR`), `eval`, and a here-document fed
//!    to a shell are at least Ask.
//! 4. Resolve every path argument and redirect target against the scope. A write inside the scope
//!    or outside `$HOME` is Allow; one under `$HOME` and outside the scope is Ask.
//! 5. The class of a compound command is the strictest class of its parts.
//! 6. A program with no rule is Allow.
//! 7. The caller logs the class and the rule.
//!
//! The cases both implementations are held to are in
//! `Packs/security/extensions/command-policy/cases.toml`, which the pi extension and
//! `libs/agent/tests/policy_cases.rs` read as the same table.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

/// What may happen to a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Refused. Only a deliberate card that names the rule can allow one call.
    Block,
    /// The approval card, and a session grant is possible.
    Ask,
    /// Runs without a card.
    Allow,
}

impl Class {
    /// Higher wins, which is how a compound command takes the strictest class of its parts.
    fn rank(self) -> u8 {
        match self {
            Class::Allow => 0,
            Class::Ask => 1,
            Class::Block => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Class::Block => "block",
            Class::Ask => "ask",
            Class::Allow => "allow",
        }
    }

    pub fn parse(text: &str) -> Option<Class> {
        match text {
            "block" => Some(Class::Block),
            "ask" => Some(Class::Ask),
            "allow" => Some(Class::Allow),
            _ => None,
        }
    }
}

/// One decision, with the rule that made it so a card and the call log can name it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub class: Class,
    /// The rule ID from the plan's table: `B1`, `A2`, `R3`, `A6`.
    pub rule: &'static str,
    /// One line for the card. Never the whole command: the card already shows that.
    pub reason: String,
}

impl Decision {
    fn new(class: Class, rule: &'static str, reason: impl Into<String>) -> Self {
        Self {
            class,
            rule,
            reason: reason.into(),
        }
    }

    fn allow(rule: &'static str, reason: impl Into<String>) -> Self {
        Self::new(Class::Allow, rule, reason)
    }

    fn ask(rule: &'static str, reason: impl Into<String>) -> Self {
        Self::new(Class::Ask, rule, reason)
    }

    fn block(rule: &'static str, reason: impl Into<String>) -> Self {
        Self::new(Class::Block, rule, reason)
    }
}

/// The strictest of the parts, which is the class of a compound command. A tie keeps the first,
/// so the rule a card names is the first part that reached the class.
fn strictest(decisions: Vec<Decision>) -> Decision {
    let mut decisions = decisions.into_iter();
    let Some(mut worst) = decisions.next() else {
        return Decision::allow("R1", "reads only");
    };
    for decision in decisions {
        if decision.class.rank() > worst.class.rank() {
            worst = decision;
        }
    }
    worst
}

/// How deep a nested command may go before the rest is refused as unreadable.
const MAX_DEPTH: usize = 8;

/// The shells whose own options decide whether they run a string.
const SHELLS: &[&str] = &["sh", "bash", "zsh", "dash", "ksh"];

/// Everything that runs a program written somewhere else.
const INTERPRETERS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "fish", "python", "python3", "node", "ruby", "perl",
];

/// The control words that begin or end a compound statement: the commands inside are classified
/// alone, so the word itself is skipped.
const CONTROL_WORDS: &[&str] = &[
    "{", "}", "!", "then", "do", "else", "elif", "fi", "done", "if", "while", "until", "esac",
];

/// The words that count as a read (`R1`).
const READS: &[&str] = &[
    "ls",
    "cat",
    "head",
    "tail",
    "wc",
    "rg",
    "grep",
    "egrep",
    "fgrep",
    "fd",
    "tree",
    "stat",
    "file",
    "du",
    "df",
    "pwd",
    "echo",
    "printf",
    "which",
    "type",
    "date",
    "sort",
    "uniq",
    "cut",
    "tr",
    "nl",
    "column",
    "diff",
    "cmp",
    "comm",
    "md5sum",
    "sha1sum",
    "sha256sum",
    "sha512sum",
    "cksum",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "jq",
    "true",
    "false",
    "test",
    "[",
    "cd",
    "pushd",
    "popd",
    "uname",
    "id",
    "whoami",
    "hostname",
    "ps",
    "uptime",
    "free",
    "nproc",
    "sleep",
    "seq",
    "xxd",
    "hexdump",
    "od",
    "strings",
    "bat",
    "eza",
    "man",
    "export",
    "set",
    "unset",
    "local",
    "read",
    "wait",
    "exit",
    "return",
    "shift",
    "trap",
    ":",
    "for",
    "case",
    "in",
];

/// `git` subcommands that only read (`R2`).
const GIT_READS: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "blame",
    "rev-parse",
    "ls-files",
    "grep",
    "shortlog",
    "describe",
    "cat-file",
    "ls-tree",
    "rev-list",
    "merge-base",
    "name-rev",
    "for-each-ref",
    "show-ref",
    "check-ignore",
    "whatchanged",
    "help",
    "version",
];

/// `git` subcommands that touch only the local index or a fresh copy (`R2`).
const GIT_LOCAL: &[&str] = &["fetch", "add", "rm", "mv", "clone", "init"];

/// `git` subcommands that change history, branches or remotes (`A2`).
const GIT_ASK: &[&str] = &[
    "commit",
    "push",
    "merge",
    "rebase",
    "cherry-pick",
    "stash",
    "tag",
    "branch",
    "switch",
    "checkout",
    "pull",
    "apply",
    "am",
    "revert",
    "reset",
    "restore",
    "submodule",
    "worktree",
    "notes",
    "config",
    "remote",
    "gc",
    "prune",
    "bisect",
    "update-index",
];

/// The `curl` flags that send a body somewhere (`A4`).
const CURL_SENDS: &[&str] = &[
    "-d",
    "-F",
    "-T",
    "--form",
    "--form-string",
    "--json",
    "--upload-file",
    "--post-data",
    "--post-file",
    "--body-data",
    "--body-file",
];

/// Programs that open a session on another host (`A4`).
const REMOTE_SHELLS: &[&str] = &["ssh", "scp", "sftp", "nc", "ncat", "telnet", "ftp"];

/// Programs that affect a running process or a container (`A5`).
const PROCESSES: &[&str] = &["kill", "pkill", "killall", "docker", "podman", "orb"];

/// Container subcommands that only read (`R1`).
const DOCKER_READS: &[&str] = &[
    "ps", "images", "logs", "inspect", "version", "info", "stats", "top",
];

/// Programs that only write, so their arguments are write targets.
const WRITERS: &[&str] = &[
    "cp", "mv", "mkdir", "touch", "ln", "tee", "install", "truncate", "rmdir", "unlink", "chmod",
    "chown", "chgrp",
];

/// Redirect targets that write to nothing anyone keeps.
const NO_WRITE_TARGETS: &[&str] = &["/dev/null", "/dev/stdout", "/dev/stderr", "/dev/tty"];

/// Wrapper options that consume the next argument.
fn wrapper_arguments(program: &str) -> &'static [&'static str] {
    match program {
        "env" => &["-u", "--unset", "-C", "--chdir", "-S", "--split-string"],
        "nice" => &["-n", "--adjustment"],
        "timeout" => &["-s", "--signal", "-k", "--kill-after"],
        "stdbuf" => &["-i", "-o", "-e"],
        "xargs" => &[
            "-n",
            "-I",
            "-P",
            "-d",
            "-L",
            "-E",
            "-s",
            "-a",
            "--max-args",
            "--max-procs",
            "--delimiter",
            "--replace",
            "--arg-file",
        ],
        _ => &[],
    }
}

/// The rule a program's bare name carries when it is reached by any route.
fn block_rule(program: &str) -> Option<&'static str> {
    Some(match program {
        "rm" => "B1",
        "shred" | "dd" | "fdisk" | "sfdisk" | "parted" | "wipefs" | "diskutil" | "csrutil"
        | "spctl" | "nvram" | "launchctl" | "systemctl" => "B5",
        "sudo" | "su" | "doas" | "pkexec" => "B3",
        _ => return None,
    })
}

/// A package manager and the subcommands that change packages or run downloaded code (`A3`).
/// `"*"` means every subcommand.
fn package_changes(program: &str) -> Option<&'static [&'static str]> {
    Some(match program {
        "npm" => &[
            "install",
            "i",
            "ci",
            "add",
            "remove",
            "rm",
            "uninstall",
            "un",
            "update",
            "up",
            "upgrade",
            "link",
            "exec",
            "x",
        ],
        "pnpm" => &[
            "install",
            "i",
            "add",
            "remove",
            "rm",
            "uninstall",
            "update",
            "up",
            "upgrade",
            "link",
            "dlx",
            "exec",
        ],
        "yarn" => &["install", "add", "remove", "upgrade", "up", "link", "dlx"],
        "bun" => &[
            "install", "i", "add", "a", "remove", "rm", "update", "upgrade", "link", "x",
        ],
        "brew" => &[
            "install",
            "reinstall",
            "upgrade",
            "uninstall",
            "remove",
            "rm",
            "tap",
            "untap",
            "link",
            "unlink",
            "cleanup",
            "update",
        ],
        "pip" | "pip3" => &["install", "uninstall", "download"],
        "uv" => &["add", "remove", "sync", "pip", "tool", "lock", "self"],
        "cargo" => &["install", "uninstall", "add", "remove", "update"],
        "rustup" => &[
            "install",
            "uninstall",
            "update",
            "default",
            "toolchain",
            "target",
            "component",
            "override",
            "self",
            "set",
        ],
        "gem" => &["install", "uninstall", "update"],
        "go" => &["install", "get"],
        "npx" | "bunx" | "uvx" | "pipx" | "apt" | "apt-get" | "dnf" | "yum" | "snap"
        | "flatpak" => &["*"],
        "dpkg" => &["-i", "--install", "-r", "--remove", "-P", "--purge"],
        _ => return None,
    })
}

/// The tier's scope.
#[derive(Clone, Debug)]
struct Scope {
    /// The directory a session may write inside without a card (the tier root).
    cwd: PathBuf,
    /// The user's home directory, which A1 turns into a card.
    home: PathBuf,
    /// Directories outside the tier root a write may still target: temp space.
    writable: Vec<PathBuf>,
    /// Whether this tier writes at all. The `sjel` tier does not.
    writes: bool,
}

/// Where a session's commands are judged.
///
/// `Policy::new` reads `$HOME` and the temp directory; the fixture that holds both
/// implementations to one table sets them instead, so a case is the same on every machine.
#[derive(Clone, Debug)]
pub struct Policy {
    scope: Scope,
}

impl Policy {
    /// A session whose writes are confined to `reach`. `None` is a tier that writes nothing.
    pub fn new(reach: Option<PathBuf>) -> Self {
        let home = home();
        let writes = reach.is_some();
        Self {
            scope: Scope {
                cwd: reach.unwrap_or_else(|| home.clone()),
                home,
                writable: temp_dirs(),
                writes,
            },
        }
    }

    /// Judge every case against this home directory rather than the machine's.
    pub fn with_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.scope.home = home.into();
        self
    }

    /// Replace the directories a write may target outside the tier root.
    pub fn with_writable(mut self, writable: Vec<PathBuf>) -> Self {
        self.scope.writable = writable;
        self
    }

    /// Judge relative paths against this directory rather than the tier root.
    pub fn in_directory(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.scope.cwd = cwd.into();
        self
    }

    /// The scope a card writes into, when there is one.
    pub fn reach(&self) -> Option<&Path> {
        self.scope.writes.then_some(self.scope.cwd.as_path())
    }

    /// One `bash` command, from its whole string to one decision.
    pub fn bash(&self, command: &str) -> Decision {
        self.classify(command, 0)
    }

    /// One `write` or `edit` target, judged by step 4 like a shell write.
    pub fn write(&self, path: &str) -> Decision {
        if !self.scope.writes {
            return Decision::block("B1", "this session may not write files");
        }
        let resolved = self.resolve_path(path);
        if self.home_outside_scope(&resolved) {
            return Decision::ask(
                "A1",
                format!("{path} is under the home directory outside the working directory"),
            );
        }
        let place = if self.in_scope(&resolved) {
            "inside"
        } else {
            "outside"
        };
        Decision::allow("R4", format!("{path} is {place} the working directory"))
    }

    /// The whole command string: one class, the strictest of its parts.
    fn classify(&self, command: &str, depth: usize) -> Decision {
        if depth > MAX_DEPTH {
            return Decision::ask("A6", "the command nests deeper than this policy reads");
        }
        let commands = parse(lex(command));
        if commands.is_empty() {
            return Decision::allow("R1", "an empty command");
        }
        strictest(
            commands
                .iter()
                .map(|command| self.classify_simple(command, depth))
                .collect(),
        )
    }

    /// One simple command and its redirects.
    fn classify_simple(&self, command: &Simple, depth: usize) -> Decision {
        let mut parts = vec![self.classify_words(&command.words, command.piped, false, depth)];
        let program = command
            .words
            .iter()
            .find(|word| !is_assignment(&word.text))
            .map(|word| strip_program(&word.text).to_owned())
            .unwrap_or_default();
        for redirect in &command.redirects {
            match redirect.op.as_str() {
                "<(" | ">(" => {
                    // A substitution into an interpreter is download-and-run.
                    if INTERPRETERS.contains(&program.as_str())
                        || program == "source"
                        || program == "."
                    {
                        parts.push(Decision::block(
                            "B4",
                            format!("process substitution executed by {program}"),
                        ));
                    }
                    parts.push(self.classify(redirect.body.as_deref().unwrap_or(""), depth + 1));
                    continue;
                }
                "<<" | "<<-" => {
                    if SHELLS.contains(&program.as_str()) {
                        parts
                            .push(self.classify(redirect.body.as_deref().unwrap_or(""), depth + 1));
                    }
                    continue;
                }
                "<" | "<<<" | "<&" => continue,
                ">&" if is_fd_only(redirect.arg.as_ref()) => continue,
                _ => {}
            }
            if let Some(argument) = &redirect.arg {
                for substitution in &argument.substitutions {
                    parts.push(self.classify(substitution, depth + 1));
                }
                let name = if program.is_empty() {
                    "redirect".to_owned()
                } else {
                    program.clone()
                };
                if let Some(decision) = self.write_target(argument, &name) {
                    parts.push(decision);
                }
            }
        }
        strictest(parts)
    }

    /// One simple command given as words. `unknown_arguments` marks a command reached through
    /// `xargs` or `find -exec`, whose arguments this policy cannot see.
    fn classify_words(
        &self,
        words: &[Word],
        piped: bool,
        unknown_arguments: bool,
        depth: usize,
    ) -> Decision {
        if depth > MAX_DEPTH {
            return Decision::ask("A6", "the command nests deeper than this policy reads");
        }

        // Assignments before the program: `VAR=value cmd`.
        let mut rest = words;
        while rest.first().is_some_and(|word| is_assignment(&word.text)) {
            let substitutions = rest[0].substitutions.clone();
            if !substitutions.is_empty() {
                let inner = strictest(
                    substitutions
                        .iter()
                        .map(|substitution| self.classify(substitution, depth + 1))
                        .collect(),
                );
                if inner.class != Class::Allow {
                    return inner;
                }
            }
            rest = &rest[1..];
        }
        let Some(head) = rest.first() else {
            return Decision::allow("R1", "an assignment");
        };

        // 3. A program name that is computed at run time.
        if head.dynamic {
            let inner: Vec<Decision> = head
                .substitutions
                .iter()
                .map(|substitution| self.classify(substitution, depth + 1))
                .collect();
            let joined = head.substitutions.join(" ");
            let reached = joined
                .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')))
                .map(strip_program)
                .find(|name| block_rule(name).is_some());
            if let Some(name) = reached {
                let rule = block_rule(name).unwrap_or("A6");
                return Decision::block(
                    rule,
                    format!("the program name is built at run time and reaches `{name}`"),
                );
            }
            let mut all = vec![Decision::ask(
                "A6",
                "the program name is computed at run time",
            )];
            all.extend(inner);
            return strictest(all);
        }

        let program = if head.text == "tools/doctor" || head.text == "./tools/doctor" {
            "tools/doctor"
        } else {
            strip_program(&head.text)
        };
        let arguments = &rest[1..];
        let texts: Vec<&str> = arguments.iter().map(|word| word.text.as_str()).collect();

        // A substitution inside an argument runs too.
        let argument_substitutions: Vec<Decision> = arguments
            .iter()
            .flat_map(|word| word.substitutions.iter())
            .map(|substitution| self.classify(substitution, depth + 1))
            .collect();
        let with_substitutions =
            |decision: Decision| side_effects(decision, &argument_substitutions);

        // 2. Wrappers that only run another command.
        if matches!(
            program,
            "env"
                | "nice"
                | "nohup"
                | "time"
                | "timeout"
                | "command"
                | "builtin"
                | "exec"
                | "stdbuf"
        ) {
            if program == "command" && matches!(texts.first(), Some(&"-v") | Some(&"-V")) {
                return Decision::allow("R1", "`command` looks a program up");
            }
            let takes_argument = wrapper_arguments(program);
            let mut index = 0;
            while index < arguments.len() {
                let text = arguments[index].text.as_str();
                if program == "env" && is_assignment(text) {
                    index += 1;
                    continue;
                }
                if program == "env"
                    && matches!(text, "-S" | "--split-string")
                    && arguments.get(index + 1).is_some()
                {
                    return with_substitutions(
                        self.classify(&arguments[index + 1].text, depth + 1),
                    );
                }
                if text.starts_with('-') {
                    index += if takes_argument.contains(&text) { 2 } else { 1 };
                    continue;
                }
                break;
            }
            // `timeout <duration> <command>`.
            if program == "timeout" {
                index += 1;
            }
            let Some(run) = arguments.get(index..) else {
                return Decision::allow("R1", format!("`{program}` runs nothing"));
            };
            if run.is_empty() {
                return Decision::allow("R1", format!("`{program}` runs nothing"));
            }
            return with_substitutions(self.classify_words(
                run,
                piped,
                unknown_arguments,
                depth + 1,
            ));
        }
        if program == "busybox" && !arguments.is_empty() {
            return self.classify_words(arguments, piped, unknown_arguments, depth + 1);
        }
        if program == "xargs" {
            let mut index = 0;
            while index < arguments.len() && arguments[index].text.starts_with('-') {
                index += if wrapper_arguments("xargs").contains(&arguments[index].text.as_str()) {
                    2
                } else {
                    1
                };
            }
            let Some(run) = arguments.get(index..) else {
                return Decision::allow("R1", "`xargs` runs nothing");
            };
            if run.is_empty() {
                return Decision::allow("R1", "`xargs` runs nothing");
            }
            return self.classify_words(run, piped, true, depth + 1);
        }
        if program == "find" {
            if texts.contains(&"-delete") {
                return Decision::block("B1", "`find -delete` removes files");
            }
            if let Some(position) = texts
                .iter()
                .position(|text| matches!(*text, "-exec" | "-execdir" | "-ok" | "-okdir"))
            {
                let end = texts
                    .iter()
                    .enumerate()
                    .skip(position + 1)
                    .find(|(_, text)| matches!(**text, ";" | "+" | "\\;"))
                    .map(|(index, _)| index);
                let inner: Vec<Word> = arguments[position + 1..end.unwrap_or(arguments.len())]
                    .iter()
                    .filter(|word| word.text != "{}")
                    .cloned()
                    .collect();
                if inner.is_empty() {
                    return Decision::ask("A6", "`find -exec` with no command this policy reads");
                }
                return self.classify_words(&inner, false, true, depth + 1);
            }
            if let Some(position) = texts
                .iter()
                .position(|text| matches!(*text, "-fprint" | "-fprint0" | "-fprintf" | "-fls"))
            {
                if let Some(target) = arguments.get(position + 1) {
                    if let Some(decision) = self.write_target(target, "find") {
                        return decision;
                    }
                }
            }
            return Decision::allow("R1", "`find` reads");
        }
        if SHELLS.contains(&program) {
            let string_at = texts
                .iter()
                .position(|text| *text == "-c" || is_combined_c_flag(text));
            if let Some(index) = string_at {
                if let Some(script) = arguments.get(index + 1) {
                    if script.dynamic {
                        return Decision::ask(
                            "A6",
                            format!("`{program} -c` runs a string built at run time"),
                        );
                    }
                    return with_substitutions(self.classify(&script.text, depth + 1));
                }
            }
        }
        if INTERPRETERS.contains(&program) {
            let runs_a_script = texts.iter().any(|text| {
                !text.starts_with('-') || matches!(*text, "-c" | "-e" | "-m" | "--eval")
            });
            if piped && !runs_a_script {
                return Decision::block(
                    "B4",
                    format!("`{program}` would run a stream this machine downloaded"),
                );
            }
            return with_substitutions(Decision::allow("R4", format!("`{program}` runs code")));
        }
        if program == "eval" {
            let mut all = vec![Decision::ask(
                "A6",
                "`eval` runs a string this policy cannot read",
            )];
            all.push(self.classify(&texts.join(" "), depth + 1));
            return with_substitutions(strictest(all));
        }
        if program == "source" || program == "." {
            return with_substitutions(Decision::allow(
                "R4",
                "`source` runs a file this policy has not read",
            ));
        }

        // The rules themselves: the harmful classes ask or block, everything else runs.
        if block_rule(program) == Some("B3") {
            return Decision::block("B3", format!("`{program}` gains privileges"));
        }
        if program == "osascript" && texts.join(" ").contains("administrator privileges") {
            return Decision::block("B3", "`osascript` with administrator privileges");
        }
        if program == "rm" {
            return with_substitutions(self.classify_rm(arguments, unknown_arguments));
        }
        if program == "git" {
            let git = self.classify_git(arguments);
            return with_substitutions(if unknown_arguments {
                strictest(vec![
                    Decision::ask("A6", "`git` with arguments chosen at run time"),
                    git,
                ])
            } else {
                git
            });
        }
        if program == "gh" {
            return with_substitutions(classify_gh(&texts));
        }
        if block_rule(program) == Some("B5") || is_mkfs(program) {
            return Decision::block("B5", format!("`{program}` changes disks or system state"));
        }
        if program == "crontab" && texts.contains(&"-r") {
            return Decision::block("B5", "`crontab -r` removes the schedule");
        }
        if program == "defaults" && texts.first() == Some(&"write") {
            return Decision::block("B5", "`defaults write` changes settings");
        }
        if matches!(program, "chmod" | "chown" | "chgrp") && short_flags(arguments).contains("-R") {
            return Decision::block("B5", format!("recursive `{program}`"));
        }
        if program == "security"
            && (texts.first().copied().is_some_and(is_password_lookup) && texts.contains(&"-w")
                || texts
                    .first()
                    .is_some_and(|action| action.starts_with("delete-")))
        {
            return Decision::block("B6", "`security` would read or delete a credential");
        }
        if program == "secret-tool" && matches!(texts.first(), Some(&"lookup") | Some(&"clear")) {
            return Decision::block("B6", "`secret-tool` would read or clear a secret");
        }
        if matches!(program, "cargo" | "npm" | "bun" | "pnpm" | "yarn" | "uv")
            && texts.first() == Some(&"publish")
        {
            return Decision::block("B7", format!("`{program} publish` is irreversible"));
        }
        if program == "twine" && texts.first() == Some(&"upload") {
            return Decision::block("B7", "`twine upload` publishes");
        }

        if let Some(changes) = package_changes(program) {
            let changes_subcommand = texts
                .iter()
                .find(|text| !text.starts_with('-') || changes.contains(text))
                .copied()
                .unwrap_or_default();
            let named = if changes_subcommand.is_empty() {
                program.to_owned()
            } else {
                format!("{program} {changes_subcommand}")
            };
            if changes.contains(&"*") || changes.contains(&changes_subcommand) {
                return Decision::ask(
                    "A3",
                    format!("`{named}` changes packages or runs downloaded code"),
                );
            }
            // A bare `bun`, `npm`, `pnpm` or `yarn` installs.
            if changes_subcommand.is_empty() && matches!(program, "bun" | "npm" | "pnpm" | "yarn") {
                return Decision::ask("A3", format!("`{named}` installs"));
            }
            return with_substitutions(Decision::allow("R3", format!("`{named}` builds or tests")));
        }
        if program == "tools/doctor" {
            return Decision::allow("R3", "`tools/doctor` reads");
        }
        if program == "sjel"
            && matches!(
                texts.first(),
                None | Some(&"help")
                    | Some(&"search")
                    | Some(&"context")
                    | Some(&"status")
                    | Some(&"--help")
            )
        {
            return Decision::allow("R3", "a `sjel` read verb");
        }
        if program == "curl" || program == "wget" {
            let mut parts: Vec<Decision> = Vec::new();
            let output = arguments.iter().position(|word| {
                matches!(
                    word.text.as_str(),
                    "-o" | "--output" | "-O" | "--output-document"
                )
            });
            if let Some(index) = output {
                let flag = arguments[index].text.as_str();
                if !(program == "curl" && flag == "-O") {
                    if let Some(target) = arguments.get(index + 1) {
                        if let Some(decision) = self.write_target(target, program) {
                            parts.push(decision);
                        }
                    }
                }
            }
            parts.push(if sends_data(&texts) {
                Decision::ask("A4", format!("`{program}` sends data"))
            } else {
                Decision::allow("R1", format!("`{program}` reads"))
            });
            return with_substitutions(strictest(parts));
        }
        if REMOTE_SHELLS.contains(&program) {
            return with_substitutions(Decision::ask(
                "A4",
                format!("`{program}` reaches a remote host"),
            ));
        }
        if program == "rsync" {
            let operands: Vec<&Word> = arguments
                .iter()
                .filter(|word| !word.text.starts_with('-'))
                .collect();
            if operands.iter().any(|word| is_remote_path(&word.text)) {
                return with_substitutions(Decision::ask(
                    "A4",
                    "`rsync` sends to or reads from a remote host",
                ));
            }
            let mut parts = vec![Decision::allow("R4", "`rsync` stays on this machine")];
            if let Some(destination) = operands.last() {
                if let Some(decision) = self.write_target(destination, program) {
                    parts.push(decision);
                }
            }
            return with_substitutions(strictest(parts));
        }
        if PROCESSES.contains(&program) {
            if matches!(program, "docker" | "podman")
                && texts
                    .first()
                    .is_some_and(|action| DOCKER_READS.contains(action))
            {
                return Decision::allow("R1", format!("`{program}` reads"));
            }
            return with_substitutions(Decision::ask(
                "A5",
                format!("`{program}` affects processes or containers"),
            ));
        }
        if WRITERS.contains(&program) {
            let targets: Vec<&Word> = arguments
                .iter()
                .filter(|word| !word.text.starts_with('-'))
                .collect();
            // `cp`, `mv`, `ln` and `install` write their last argument; `mv` also removes its
            // sources.
            let mut written: Vec<&Word> = match program {
                "cp" | "mv" | "ln" | "install" => targets.iter().rev().take(1).copied().collect(),
                _ => targets.clone(),
            };
            if program == "mv" {
                written.extend(targets.iter().rev().skip(1).copied());
            }
            let mut parts = vec![Decision::allow("R4", format!("`{program}` writes files"))];
            for target in written {
                if let Some(decision) = self.write_target(target, program) {
                    parts.push(decision);
                }
            }
            return with_substitutions(strictest(parts));
        }
        if READS.contains(&program) {
            return with_substitutions(Decision::allow("R1", format!("`{program}` only reads")));
        }
        with_substitutions(Decision::allow(
            "R4",
            format!("`{program}` has no rule, so it runs"),
        ))
    }

    /// `rm` and where it points: never the scope, the home directory, a root, or outside.
    fn classify_rm(&self, arguments: &[Word], unknown_arguments: bool) -> Decision {
        if unknown_arguments {
            return Decision::block("B1", "`rm` with arguments chosen at run time");
        }
        let flags = short_flags(arguments);
        if ["-r", "-R", "-f", "--recursive", "--force"]
            .iter()
            .any(|flag| flags.contains(*flag))
        {
            return Decision::block("B1", "`rm` with a recursive or forced flag");
        }
        for target in arguments.iter().filter(|word| !word.text.starts_with('-')) {
            if target.dynamic {
                return Decision::ask("A6", "`rm` of a path only known when it runs");
            }
            let path = self.resolve_path(&glob_root(&target.text));
            if path == Path::new("/")
                || path == canonical(&self.scope.home)
                || path == canonical(&self.scope.cwd)
                || !self.in_scope(&path)
            {
                return Decision::block("B1", format!("`rm` of {}", target.text));
            }
        }
        Decision::allow("R4", "`rm` of a file in the working directory")
    }

    /// `git` and its subcommand: history that cannot be recovered is Block, a change is Ask.
    fn classify_git(&self, arguments: &[Word]) -> Decision {
        let mut index = 0;
        // Global options before the subcommand.
        while index < arguments.len() && arguments[index].text.starts_with('-') {
            if matches!(
                arguments[index].text.as_str(),
                "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace"
            ) {
                index += 2;
            } else {
                index += 1;
            }
        }
        let subcommand = arguments
            .get(index)
            .map(|word| word.text.clone())
            .unwrap_or_default();
        let rest: Vec<&str> = arguments
            .iter()
            .skip(index + 1)
            .map(|word| word.text.as_str())
            .collect();
        let flags = short_flags(
            &arguments
                .iter()
                .skip(index + 1)
                .cloned()
                .collect::<Vec<Word>>(),
        );
        let has = |wanted: &[&str]| {
            wanted
                .iter()
                .any(|flag| flags.contains(*flag) || rest.contains(flag))
        };
        if subcommand.is_empty() {
            return Decision::allow("R2", "`git` without a subcommand");
        }
        if subcommand == "reset" && has(&["--hard", "--merge", "--keep"]) {
            return Decision::block("B2", "`git reset` discards work");
        }
        if subcommand == "clean" && has(&["-f", "--force"]) {
            return Decision::block("B2", "`git clean` deletes untracked files");
        }
        if subcommand == "checkout"
            && (rest.contains(&"--") || rest.contains(&".") || has(&["-f", "--force"]))
        {
            return Decision::block("B2", "`git checkout` overwrites worktree files");
        }
        if subcommand == "restore" && !(has(&["--staged", "-S"]) && !has(&["--worktree", "-W"])) {
            return Decision::block("B2", "`git restore` overwrites worktree files");
        }
        if subcommand == "push"
            && (has(&[
                "-f",
                "--force",
                "--force-with-lease",
                "--force-if-includes",
                "--mirror",
                "-d",
                "--delete",
                "--prune",
            ]) || rest.iter().any(|argument| {
                argument.starts_with('+')
                    || (argument.starts_with(':') && argument.len() > 1)
                    || argument.starts_with("--force")
            }))
        {
            return Decision::block("B2", "`git push` rewrites or deletes remote history");
        }
        if subcommand == "branch"
            && (has(&["-D"])
                || (has(&["-d", "--delete"]) && has(&["-f", "--force"]))
                || has(&["-M", "-C"]))
        {
            return Decision::block("B2", "`git branch` force-deletes or overwrites a branch");
        }
        if subcommand == "tag" && has(&["-d", "--delete", "-f", "--force"]) {
            return Decision::block("B2", "`git tag` deletes or moves a tag");
        }
        if subcommand == "filter-repo" || subcommand == "filter-branch" {
            return Decision::block("B2", format!("`git {subcommand}` rewrites history"));
        }
        if subcommand == "reflog" && matches!(rest.first(), Some(&"expire") | Some(&"delete")) {
            return Decision::block("B2", "`git reflog` loses history");
        }
        if subcommand == "stash" && matches!(rest.first(), Some(&"drop") | Some(&"clear")) {
            return Decision::block("B2", "`git stash` drops saved work");
        }
        if subcommand == "update-ref" && has(&["-d"]) {
            return Decision::block("B2", "`git update-ref` deletes a ref");
        }
        if GIT_READS.contains(&subcommand.as_str()) {
            return Decision::allow("R2", format!("`git {subcommand}` reads"));
        }
        let listing = |allowed: &[&str]| {
            rest.iter().all(|argument| {
                allowed.contains(argument)
                    || argument.starts_with("--format")
                    || argument.starts_with("--sort")
                    || argument.starts_with("--contains")
                    || argument.starts_with("--merged")
            })
        };
        if subcommand == "branch"
            && listing(&[
                "-a",
                "-r",
                "-l",
                "--list",
                "-v",
                "-vv",
                "--show-current",
                "--all",
                "--remotes",
            ])
        {
            return Decision::allow("R2", "`git branch` lists");
        }
        if subcommand == "tag" && (rest.is_empty() || has(&["-l", "--list"])) {
            return Decision::allow("R2", "`git tag` lists");
        }
        if subcommand == "remote"
            && (rest.is_empty() || rest.iter().all(|a| matches!(*a, "-v" | "show" | "get-url")))
        {
            return Decision::allow("R2", "`git remote` lists");
        }
        if subcommand == "config" && has(&["--get", "--get-all", "--list", "-l", "--get-regexp"]) {
            return Decision::allow("R2", "`git config` reads");
        }
        if subcommand == "stash" && matches!(rest.first(), Some(&"list") | Some(&"show")) {
            return Decision::allow("R2", "`git stash` reads");
        }
        if subcommand == "reflog" {
            return Decision::allow("R2", "`git reflog` reads");
        }
        if subcommand == "restore" && has(&["--staged", "-S"]) {
            return Decision::allow("R2", "`git restore --staged` reads the index");
        }
        if GIT_LOCAL.contains(&subcommand.as_str()) {
            return Decision::allow("R2", format!("`git {subcommand}`"));
        }
        if GIT_ASK.contains(&subcommand.as_str()) {
            return Decision::ask(
                "A2",
                format!("`git {subcommand}` changes history, branches or remotes"),
            );
        }
        Decision::allow("R4", format!("`git {subcommand}` has no rule"))
    }

    /// A write to `raw`: Ask under `$HOME` outside the scope, otherwise Allow (step 4).
    fn write_target(&self, raw: &Word, program: &str) -> Option<Decision> {
        if !self.scope.writes {
            return Some(Decision::block("B1", "this session may not write files"));
        }
        if raw.dynamic {
            return Some(Decision::ask(
                "A1",
                format!("`{program}` writes a path only known when it runs"),
            ));
        }
        if NO_WRITE_TARGETS.contains(&raw.text.as_str()) {
            return None;
        }
        let path = self.resolve_path(&glob_root(&raw.text));
        if self.home_outside_scope(&path) {
            return Some(Decision::ask(
                "A1",
                format!(
                    "`{program}` writes under the home directory outside the working directory ({})",
                    raw.text
                ),
            ));
        }
        Some(Decision::allow(
            "R4",
            format!("`{program}` writes {}", raw.text),
        ))
    }

    /// A path argument as the shell would see it, with `~`, `..` and a relative path resolved
    /// against the working directory, and the existing part of it followed through symlinks.
    fn resolve_path(&self, raw: &str) -> PathBuf {
        let raw = glob_root(raw);
        let expanded = if raw == "~" {
            self.scope.home.clone()
        } else if let Some(rest) = raw.strip_prefix("~/") {
            self.scope.home.join(rest)
        } else {
            PathBuf::from(&raw)
        };
        let joined = if expanded.is_absolute() {
            expanded
        } else {
            self.scope.cwd.join(expanded)
        };
        canonical(&normalize(&joined))
    }

    fn in_scope(&self, path: &Path) -> bool {
        std::iter::once(&self.scope.cwd)
            .chain(self.scope.writable.iter())
            .any(|root| within(path, root))
    }

    /// Under the home directory but outside the scope: the one write that asks (A1).
    fn home_outside_scope(&self, path: &Path) -> bool {
        !self.in_scope(path) && within(path, &self.scope.home)
    }
}

/// The strictest of a decision and the substitutions that ran alongside it.
fn side_effects(decision: Decision, substitutions: &[Decision]) -> Decision {
    let mut all = vec![decision];
    all.extend(substitutions.iter().cloned());
    strictest(all)
}

/// `gh` and its area: publishing is Block, a write to GitHub is Ask, a read is Allow.
fn classify_gh(arguments: &[&str]) -> Decision {
    let area = arguments.first().copied().unwrap_or_default();
    let action = arguments.get(1).copied().unwrap_or_default();
    if area == "release" && action == "create" {
        return Decision::block("B7", "`gh release create` publishes");
    }
    if area == "repo" && action == "delete" {
        return Decision::block("B7", "`gh repo delete` is irreversible");
    }
    if area == "api" {
        let sends = arguments.iter().any(|argument| {
            matches!(
                *argument,
                "-f" | "-F" | "--field" | "--raw-field" | "--input"
            )
        });
        let method_writes = arguments.iter().enumerate().any(|(index, argument)| {
            index > 0
                && matches!(arguments[index - 1], "-X" | "--method")
                && !matches!(argument.to_ascii_uppercase().as_str(), "GET" | "HEAD")
        });
        return if sends || method_writes {
            Decision::ask("A4", "`gh api` writes to GitHub")
        } else {
            Decision::allow("R1", "`gh api` reads")
        };
    }
    let reads = [
        "list", "view", "diff", "checks", "status", "watch", "download",
    ];
    if reads.contains(&action) {
        return Decision::allow("R1", format!("`gh {area} {action}` reads"));
    }
    if matches!(area, "auth" | "--version" | "version" | "help") {
        return if area == "auth" && action != "status" {
            Decision::ask("A4", "`gh auth` changes credentials")
        } else {
            Decision::allow("R1", format!("`gh {area} {action}` reads"))
        };
    }
    Decision::ask("A4", format!("`gh {area} {action}` writes to GitHub"))
}

/// The path part of an argument, without a trailing glob (`src/*.rs` is `src`).
fn glob_root(text: &str) -> String {
    let cut = text
        .char_indices()
        .find(|(_, c)| matches!(c, '*' | '?' | '['))
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let trimmed = &text[..cut];
    if trimmed.is_empty() {
        ".".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The flags of a command, with `-rf` expanded to `-r` and `-f`.
fn short_flags(arguments: &[Word]) -> HashSet<String> {
    let mut flags = HashSet::new();
    for argument in arguments {
        let text = argument.text.as_str();
        if text == "--" {
            break;
        }
        if text.starts_with("--") {
            flags.insert(text.to_owned());
        } else if text.starts_with('-') && text.len() > 1 {
            for flag in text.chars().skip(1) {
                flags.insert(format!("-{flag}"));
            }
        }
    }
    flags
}

/// The program name without a leading backslash or a path: `/bin/rm` is `rm`, `tools/doctor`
/// stays itself.
fn strip_program(program: &str) -> &str {
    let program = program.strip_prefix('\\').unwrap_or(program);
    if Path::new(program).is_absolute() {
        Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(program)
    } else {
        program
    }
}

/// `NAME=value`, the assignment that may precede a program.
fn is_assignment(text: &str) -> bool {
    let Some(equals) = text.find('=') else {
        return false;
    };
    let mut name = text[..equals].chars();
    match name.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    name.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_mkfs(program: &str) -> bool {
    program == "mkfs" || program.starts_with("mkfs.")
}

/// `find-*-password` and its relatives.
fn is_password_lookup(action: &str) -> bool {
    action.starts_with("find-") && action.ends_with("-password")
}

/// `-c` or a combined flag such as `-ec`.
fn is_combined_c_flag(text: &str) -> bool {
    text.len() > 2
        && text.starts_with('-')
        && !text.starts_with("--")
        && text.chars().skip(1).any(|c| c == 'c')
}

/// `1` and `2-`: the target of `>&` that names a file descriptor rather than a file.
fn is_fd_only(argument: Option<&Word>) -> bool {
    let Some(argument) = argument else {
        return false;
    };
    argument
        .text
        .chars()
        .all(|c| c.is_ascii_digit() || c == '-')
}

/// `host:/srv` and `user@host:path`: a path that is not on this machine.
fn is_remote_path(text: &str) -> bool {
    match text.find(':') {
        Some(index) => !text[..index].contains('/'),
        None => false,
    }
}

/// Whether a `curl` or `wget` invocation sends a body (`A4`).
fn sends_data(arguments: &[&str]) -> bool {
    arguments.iter().enumerate().any(|(index, argument)| {
        let flag = argument.split('=').next().unwrap_or(argument);
        if CURL_SENDS.contains(&flag) || flag.starts_with("--data") {
            return true;
        }
        if argument.len() > 2
            && argument.starts_with('-')
            && !argument.starts_with("--")
            && argument
                .chars()
                .nth(1)
                .is_some_and(|c| c == 'd' || c == 'F')
        {
            return true;
        }
        if matches!(*argument, "-X" | "--request" | "--method") {
            let method = arguments.get(index + 1).copied().unwrap_or_default();
            return !matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD");
        }
        for prefix in ["-X", "--request=", "--method="] {
            if let Some(method) = argument.strip_prefix(prefix) {
                return !matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD");
            }
        }
        false
    })
}

// ---- lexer and parser ---------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct Word {
    text: String,
    /// The contents of every `$(…)` and backtick pair inside the word.
    substitutions: Vec<String>,
    /// The word contains `$VAR`, `${…}` or a substitution, so its value is unknown before it runs.
    dynamic: bool,
}

#[derive(Clone, Debug)]
struct Op {
    op: String,
    /// For a redirect: the target. For `<(` and `>(`: none, the contents are in `body`.
    arg: Option<Word>,
    /// For `<(` and `>(`: the command inside. For a here-document: its body.
    body: Option<String>,
}

#[derive(Clone, Debug)]
enum Token {
    Word(Word),
    Op(Op),
}

/// One simple command: its words, its redirects, and whether a pipe feeds its stdin.
#[derive(Clone, Debug)]
struct Simple {
    words: Vec<Word>,
    redirects: Vec<Op>,
    piped: bool,
}

/// The contents of a balanced `open … close`, starting just past the opening character.
fn read_balanced(source: &[char], index: usize, open: char, close: char) -> (String, usize) {
    let mut depth = 1;
    let mut cursor = index;
    let mut quote: Option<char> = None;
    while cursor < source.len() {
        let c = source[cursor];
        if let Some(q) = quote {
            if c == '\\' && q == '"' {
                cursor += 1;
            } else if c == q {
                quote = None;
            }
        } else if c == '\'' || c == '"' {
            quote = Some(c);
        } else if c == '\\' {
            cursor += 1;
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return (source[index..cursor].iter().collect(), cursor + 1);
            }
        }
        cursor += 1;
    }
    let rest = index.min(source.len());
    (source[rest..].iter().collect(), source.len())
}

/// The contents between `index` and the next `delimiter`.
fn read_delimited(source: &[char], index: usize, delimiter: char) -> (String, usize) {
    let mut cursor = index;
    while cursor < source.len() && source[cursor] != delimiter {
        cursor += 1;
    }
    (
        source[index..cursor].iter().collect(),
        if cursor < source.len() {
            cursor + 1
        } else {
            source.len()
        },
    )
}

fn is_var_start(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '{' | '@' | '*' | '#' | '?' | '$' | '!')
}

fn is_delimiter(c: char) -> bool {
    c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '<' | '>')
}

/// A leading redirect operator and how many bytes it takes.
fn redirect_operator(text: &str) -> Option<(&'static str, usize)> {
    let digits = text
        .char_indices()
        .find(|(_, c)| !c.is_ascii_digit())
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let rest = &text[digits..];
    for op in [
        "&>>", ">>", ">|", ">&", "&>", ">", "<<<", "<<-", "<<", "<&", "<>", "<",
    ] {
        if rest.starts_with(op) {
            return Some((op, digits + op.len()));
        }
    }
    None
}

/// Lex one command string with shell quoting rules.
fn lex(source: &str) -> Vec<Token> {
    let s: Vec<char> = source.chars().collect();
    let len = s.len();
    let mut tokens: Vec<Token> = Vec::new();
    let mut pending: Vec<(String, bool, usize)> = Vec::new();
    let mut word: Option<Word> = None;
    let mut index = 0;

    macro_rules! flush {
        () => {
            if let Some(word) = word.take() {
                tokens.push(Token::Word(word));
            }
        };
    }
    macro_rules! current {
        () => {
            word.get_or_insert_with(Word::default)
        };
    }

    while index < len {
        let c = s[index];

        if c == '\n' {
            flush!();
            tokens.push(Token::Op(Op {
                op: ";".into(),
                arg: None,
                body: None,
            }));
            index += 1;
            // A here-document's body starts on the line after its operator.
            while !pending.is_empty() {
                let (delimiter, strip, at) = pending.remove(0);
                let mut lines: Vec<String> = Vec::new();
                while index < len {
                    let start = index;
                    while index < len && s[index] != '\n' {
                        index += 1;
                    }
                    let line: String = s[start..index].iter().collect();
                    if index < len {
                        index += 1;
                    }
                    let compared = if strip {
                        line.trim_start_matches('\t').to_owned()
                    } else {
                        line.clone()
                    };
                    if compared == delimiter {
                        break;
                    }
                    lines.push(line);
                }
                if let Token::Op(op) = &mut tokens[at] {
                    op.body = Some(lines.join("\n"));
                }
            }
            continue;
        }
        if c == ' ' || c == '\t' {
            flush!();
            index += 1;
            continue;
        }
        if c == '#' && word.is_none() {
            while index < len && s[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if c == '\\' {
            if index + 1 < len && s[index + 1] == '\n' {
                index += 2;
            } else {
                if index + 1 < len {
                    current!().text.push(s[index + 1]);
                }
                index += 2;
            }
            continue;
        }
        if c == '\'' {
            let start = index + 1;
            let mut cursor = start;
            while cursor < len && s[cursor] != '\'' {
                cursor += 1;
            }
            let text: String = s[start..cursor].iter().collect();
            current!().text.push_str(&text);
            index = if cursor < len { cursor + 1 } else { len };
            continue;
        }
        if c == '"' {
            index += 1;
            while index < len && s[index] != '"' {
                if s[index] == '\\' && index + 1 < len {
                    current!().text.push(s[index + 1]);
                    index += 2;
                } else if s[index] == '$' && index + 1 < len && s[index + 1] == '(' {
                    let (inner, next) = read_balanced(&s, index + 2, '(', ')');
                    let word = current!();
                    word.substitutions.push(inner.clone());
                    word.dynamic = true;
                    word.text.push_str("$(");
                    word.text.push_str(&inner);
                    word.text.push(')');
                    index = next;
                } else if s[index] == '`' {
                    let (inner, next) = read_delimited(&s, index + 1, '`');
                    let word = current!();
                    word.substitutions.push(inner.clone());
                    word.dynamic = true;
                    word.text.push('`');
                    word.text.push_str(&inner);
                    word.text.push('`');
                    index = next;
                } else {
                    if s[index] == '$' && index + 1 < len && is_var_start(s[index + 1]) {
                        current!().dynamic = true;
                    }
                    current!().text.push(s[index]);
                    index += 1;
                }
            }
            index += 1;
            continue;
        }
        if c == '$' && index + 1 < len && s[index + 1] == '(' {
            let (inner, next) = read_balanced(&s, index + 2, '(', ')');
            let word = current!();
            word.substitutions.push(inner.clone());
            word.dynamic = true;
            word.text.push_str("$(");
            word.text.push_str(&inner);
            word.text.push(')');
            index = next;
            continue;
        }
        if c == '`' {
            let (inner, next) = read_delimited(&s, index + 1, '`');
            let word = current!();
            word.substitutions.push(inner.clone());
            word.dynamic = true;
            word.text.push('`');
            word.text.push_str(&inner);
            word.text.push('`');
            index = next;
            continue;
        }
        if c == '$' && index + 1 < len && is_var_start(s[index + 1]) {
            current!().dynamic = true;
            current!().text.push('$');
            index += 1;
            continue;
        }
        if (c == '<' || c == '>') && index + 1 < len && s[index + 1] == '(' {
            flush!();
            let (inner, next) = read_balanced(&s, index + 2, '(', ')');
            tokens.push(Token::Op(Op {
                op: format!("{c}("),
                arg: None,
                body: Some(inner),
            }));
            index = next;
            continue;
        }

        // Redirects, including a file-descriptor prefix that is the whole current word (`2>`).
        let prefix = match &word {
            Some(word)
                if !word.dynamic
                    && !word.text.is_empty()
                    && word.text.chars().all(|c| c.is_ascii_digit()) =>
            {
                word.text.clone()
            }
            _ => String::new(),
        };
        let candidate: String = prefix.chars().chain(s[index..].iter().copied()).collect();
        if c == '<' || c == '>' || (c == '&' && index + 1 < len && s[index + 1] == '>') {
            if let Some((op, consumed)) = redirect_operator(&candidate) {
                if !prefix.is_empty() {
                    word = None;
                }
                flush!();
                index += consumed - prefix.len();
                while index < len && (s[index] == ' ' || s[index] == '\t') {
                    index += 1;
                }
                let start = index;
                let mut cursor = index;
                let mut quote: Option<char> = None;
                while cursor < len {
                    let ch = s[cursor];
                    if let Some(q) = quote {
                        if ch == q {
                            quote = None;
                        }
                    } else if ch == '\'' || ch == '"' {
                        quote = Some(ch);
                    } else if is_delimiter(ch) {
                        break;
                    }
                    cursor += 1;
                }
                let raw: String = s[start..cursor].iter().collect();
                index = cursor;
                let target = lex_word(&raw);
                tokens.push(Token::Op(Op {
                    op: op.to_owned(),
                    arg: Some(target.clone()),
                    body: None,
                }));
                if op == "<<" || op == "<<-" {
                    pending.push((target.text, op == "<<-", tokens.len() - 1));
                }
                continue;
            }
        }

        let two: String = s[index..(index + 2).min(len)].iter().collect();
        if matches!(two.as_str(), "&&" | "||" | "|&" | ";;") {
            flush!();
            let op = match two.as_str() {
                "|&" => "|",
                ";;" => ";",
                other => other,
            };
            tokens.push(Token::Op(Op {
                op: op.to_owned(),
                arg: None,
                body: None,
            }));
            index += 2;
            continue;
        }
        if matches!(c, ';' | '|' | '&' | '(' | ')') {
            flush!();
            tokens.push(Token::Op(Op {
                op: c.to_string(),
                arg: None,
                body: None,
            }));
            index += 1;
            continue;
        }
        current!().text.push(c);
        index += 1;
    }
    flush!();
    tokens
}

/// The first word of a short fragment, used for a redirect's target.
fn lex_word(raw: &str) -> Word {
    lex(raw)
        .into_iter()
        .find_map(|token| match token {
            Token::Word(word) => Some(word),
            Token::Op(_) => None,
        })
        .unwrap_or_default()
}

/// Split the tokens into simple commands. A pipeline marks the next command as piped.
fn parse(tokens: Vec<Token>) -> Vec<Simple> {
    let mut commands: Vec<Simple> = Vec::new();
    let mut words: Vec<Word> = Vec::new();
    let mut redirects: Vec<Op> = Vec::new();
    let mut push = |words: &mut Vec<Word>, redirects: &mut Vec<Op>, piped: bool| {
        if !words.is_empty() || !redirects.is_empty() {
            commands.push(Simple {
                words: std::mem::take(words),
                redirects: std::mem::take(redirects),
                piped,
            });
        }
    };
    let mut piped = false;
    for token in tokens {
        match token {
            Token::Word(word) => {
                if words.is_empty() && CONTROL_WORDS.contains(&word.text.as_str()) {
                    continue;
                }
                words.push(word);
            }
            Token::Op(op) => match op.op.as_str() {
                "|" => {
                    push(&mut words, &mut redirects, piped);
                    piped = true;
                }
                ";" | "&&" | "||" | "&" | "(" | ")" => {
                    push(&mut words, &mut redirects, piped);
                    piped = false;
                }
                _ => redirects.push(op),
            },
        }
    }
    push(&mut words, &mut redirects, piped);
    commands
}

// ---- paths --------------------------------------------------------------------------------

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The temp directories a write may target besides the tier root.
fn temp_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/tmp")];
    if let Some(tmp) = std::env::var_os("TMPDIR") {
        dirs.push(PathBuf::from(tmp));
    }
    dirs
}

/// Follow symlinks for the part of the path that exists, keep the rest lexical.
fn canonical(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if existing.exists() {
            let mut resolved =
                std::fs::canonicalize(&existing).unwrap_or_else(|_| existing.clone());
            for part in missing.iter().rev() {
                resolved.push(part);
            }
            return normalize(&resolved);
        }
        let Some(parent) = existing.parent() else {
            return path.to_path_buf();
        };
        if parent == existing.as_path() {
            return path.to_path_buf();
        }
        missing.push(
            existing
                .file_name()
                .map(|n| n.to_os_string())
                .unwrap_or_default(),
        );
        existing = parent.to_path_buf();
    }
}

/// `..` and `.` resolved textually, so a scope check does not depend on the path existing.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Whether `path` is `root` or inside it.
fn within(path: &Path, root: &Path) -> bool {
    let root = canonical(root);
    path == root || path.starts_with(&root)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same fixture the shared table is written against.
    fn policy() -> Policy {
        Policy::new(Some(PathBuf::from("/home/u/proj")))
            .with_home("/home/u")
            .with_writable(vec![PathBuf::from("/tmp")])
    }

    #[test]
    fn a_quoting_survives_the_lexer() {
        let commands = parse(lex("git commit -m 'a b; c' && ls"));
        assert_eq!(commands.len(), 2);
        let first: Vec<&str> = commands[0]
            .words
            .iter()
            .map(|word| word.text.as_str())
            .collect();
        assert_eq!(first, ["git", "commit", "-m", "a b; c"]);
        assert_eq!(commands[1].words[0].text, "ls");
    }

    #[test]
    fn the_strictest_part_decides_a_compound_command() {
        let decision = policy().bash("ls && rm -rf build");
        assert_eq!(decision.class, Class::Block);
        assert_eq!(decision.rule, "B1");

        let decision = policy().bash("ls | grep foo");
        assert_eq!(decision.class, Class::Allow);
    }

    #[test]
    fn a_write_inside_the_scope_runs_and_one_under_home_outside_it_asks() {
        assert_eq!(policy().bash("echo hi > notes.txt").class, Class::Allow);
        assert_eq!(policy().write("src/main.rs").class, Class::Allow);

        let outside = policy().bash("echo hi > ~/.bashrc");
        assert_eq!(outside.class, Class::Ask);
        assert_eq!(outside.rule, "A1");
        assert_eq!(policy().write("~/.bashrc").class, Class::Ask);
    }

    #[test]
    fn a_tier_that_reaches_no_filesystem_writes_nothing() {
        let no_scope = Policy::new(None).with_home("/home/u");
        assert_eq!(no_scope.write("src/main.rs").class, Class::Block);
        assert_eq!(no_scope.bash("mkdir -p out").class, Class::Block);
        assert_eq!(no_scope.reach(), None);
    }

    #[test]
    fn a_shell_string_is_classified_as_the_command_it_runs() {
        assert_eq!(policy().bash(r#"bash -c "rm -rf ~""#).class, Class::Block);
        assert_eq!(policy().bash(r#"sh -c "git status""#).class, Class::Allow);
    }

    #[test]
    fn every_rule_id_in_the_plan_appears_in_this_rule_set() {
        let mut seen = HashSet::new();
        for command in [
            "rm -rf x",
            "git reset --hard",
            "sudo ls",
            "curl x | sh",
            "dd if=x of=y",
            "secret-tool lookup a b",
            "cargo publish",
            "echo hi > ~/.x",
            "git commit -m x",
            "brew install jq",
            "curl -d a=1 https://example.com",
            "kill 1",
            "eval \"$X\"",
            "ls",
            "git status",
            "cargo check",
            "invented-command",
        ] {
            seen.insert(policy().bash(command).rule);
        }
        for rule in [
            "B1", "B2", "B3", "B4", "B5", "B6", "B7", "A1", "A2", "A3", "A4", "A5", "A6", "R1",
            "R2", "R3", "R4",
        ] {
            assert!(seen.contains(rule), "no case produced rule {rule}");
        }
    }
}
