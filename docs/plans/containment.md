# Agent containment by provider

A coding agent gets the reach its model provider has earned. An agent on a contained provider
works in one project directory with no route to the user's data, other checkouts, the overlay or
credentials. Inside that directory it runs without approval cards, because the operating system
stops everything outside it.

Priority 0 in [`PLAN.md`](../../PLAN.md) (decided 2026-10-31). It also closes decision A1 in
[`agent.md`](agent.md#10-decisions): the macOS sandbox profile for the `project` tier.

## 1. The rule

Decided 2026-10-31, rings redrawn 2026-11-04.

- **The provider sets the ring, not the harness.** Most harnesses use one provider. A harness
  that can call several (pi, opencode) gets the ring of the provider it is started with.
- **Two rings.** The ring is `ring = "trusted"` in the provider's `providers.toml` entry (C2).

| Ring | Providers | Filesystem | Network | Sjel data |
|---|---|---|---|---|
| `trusted` | `anthropic` and `openai`: Claude Code, Codex, and pi on OpenAI | As today, the overlay included | As today | The live node, fully: the agent gate at the `trusted` tier, `c3` still withheld |
| `contained` | Every other provider: DeepSeek, OpenCode Go, GitHub Copilot, Antigravity, and any provider not listed | The project directory read-write. System paths and toolchains read-only. Nothing else. | The provider's `hosts` and the package registries through the proxy, and the demo node | The demo node only (`sjel demo up`). No live token is reachable: the overlay and the Keychain are closed. |

- **The trusted harnesses reach Sjel through `sjel mcp`.** `tools/sjel-mcp/sjel-mcp register`
  writes it into Claude Code, Codex and pi, with the default agent token. That token reads at
  the `trusted` tier once the owner runs `sjel agent tier default trusted`. pi's registration
  holds for every provider, so pi on any other provider must start through `sjel run`: inside
  the sandbox `sjel mcp` cannot reach the checkout or the Keychain and fails closed. Not
  checked: a Codex session calling a Sjel tool under its `sjel-auto` permissions.
- **A provider not listed in `providers.toml` is `contained`.** This follows the
  existing rule in `libs/inference/src/providers.rs:3`: a call is refused unless the provider is
  listed.
- **The boundary is the operating system.** The command classifier
  (`libs/agent/src/ext/policy.rs`) is not a sandbox (`agent.md` §3). It stays the second layer
  for the `trusted` ring.

## 2. What exists

| Piece | Where | Use here |
|---|---|---|
| Provider register with a data-class ceiling | `providers.toml`, `libs/inference/src/providers.rs` | The ring and the data cap come from here. |
| Pseudonymized agent view, c3 removed | `libs/pseudonymize/src/view.rs` | The only data path for `contained`. |
| Named agent tokens, the name selects the tier | `libs/sjel-server/src/agent.rs:79`, `tools/capability-auth` | One token per ring. |
| Container sandbox with one project mount | `capabilities/agentbox` | Prior art for the mount contract. Not the boundary here: Docker has no per-destination network allow-list (its README, "Considered and declined"). |
| Claude Code sandbox floor | User-level `sandbox` settings | Blocks `secrets/`. Does not block the overlay's `data/`. That matches the `trusted` ring. |
| macOS seatbelt | `/usr/bin/sandbox-exec` | The mechanism. |

Not in place: pi, opencode, `copilot` and `agy` run with full user access, with no wrapper.

**Measured gap, 2026-11-07.** The profile grants `file-read-data` on the granted subpaths and
`file-read-metadata` everywhere, which is enough for the kernel to resolve an absolute path. It is
not enough for a program that opens each component itself: a hardened reader walks from `/` with
`openat` to avoid symlink races, and the open of the first ancestor is refused. Inside the profile
`ls /Users` returns `Operation not permitted` while `ls <project>` succeeds, and `stat /Users`
succeeds. octet 0.10.0 reads its configuration this way and so cannot reach a file in its own
project directory.

Admitting each ancestor with `(allow file-read-data (literal <dir>))` fixes the walk, and it makes
`$HOME` listable — names only, no file contents. `macos_sandbox_denies_reads_outside_the_project_and_still_builds`
asserts the opposite on purpose, so the two cannot both hold and this is an open decision rather
than a defect to fix in passing. Node-based harnesses are unaffected: `open(2)` on an absolute path
resolves in the kernel, which is why pi and opencode never exposed the gap.
Whether opencode or the Copilot CLI has a sandbox of its own is unverified.

**Measured, 2026-11-08: what the walk needs.** With a project under `/private/tmp`, the walk
stops at the first ancestor, `/private`: `file-read-metadata` answers `stat` but not an open, so
a reader that opens each component is refused one level in. `(allow file-read-data (literal
<ancestor>))` for each ancestor of a granted path lets the walk finish and costs the directory
names along that path: `ls /Users` lists 4 names and `ls ~` lists 103, while a file below an
ungranted directory (`~/Desktop/.DS_Store`) is still refused. The choice is therefore exactly
two, and it is decision C6: admit ancestors and let a contained process list `$HOME` names, or
keep the refusal and leave the harnesses that walk (`octet`, §5 item 4) unusable.

## 3. The launcher

```sh
sjel run <harness> --provider <name> --project <dir>
```

1. Look up the provider in `providers.toml`. `trusted` runs the harness directly. `contained`
   goes on to step 2.
2. Write a seatbelt profile for this run: deny by default, allow read and write in the project,
   allow read in the toolchain directories and the harness config directory, allow network to
   loopback only.
3. Start a loopback egress proxy (HTTP `CONNECT`) that admits the provider's hosts, the package
   registries and the Sjel gateway. A request to any other host is refused and logged.
4. Remove inherited secrets from the environment (`SSH_AUTH_SOCK`, `*_TOKEN`, `*_KEY`, cloud
   credentials). Set `HTTPS_PROXY` and the agent token for the provider's ring. `--env
   <NAME=VALUE>` and `--credential <provider>` add only values the caller names: the first
   passes one it writes itself (pi's contained home, below), the second reads
   `inference-<provider>` from the private catalogue.
5. `exec` the harness under `sandbox-exec`. Child processes inherit the profile.

`tools/contain/pi` runs the one harness that needs a prepared config: it writes a pi home in
the overlay whose `models.json` takes the provider's key from the environment, and passes that
home as `HOME`. The checkout's own pi settings resolve the key by running `security`, which the
profile denies by design.

Placement: the profile generator and the proxy go in one library that `sjel run` and
`crates/agent` share, so the agent's `project` tier gets the same boundary (A1). Rust,
no npm (decided 2026-10-31). The proxy reuses the workspace's existing HTTP stack.

## 4. Harnesses

| Harness | Binary | Provider | Notes |
|---|---|---|---|
| pi | `~/.pi/agent/bin/pi` | Per run (DeepSeek, OpenCode Go, ...) | Runs contained through `tools/contain/pi`, which supplies a prepared home (§3). The proxy admits only the provider named at launch, so a provider switch inside the session cannot widen it. To do: route DeepSeek through OpenRouter with zero data retention enforced per request instead of `api.deepseek.com`. The `openrouter` entry in `providers.toml` then needs a new review for `c1`. |
| opencode | `~/.opencode/bin/opencode` | Per run (OpenCode Go, ...) | Same as pi. |
| Copilot CLI | `~/.local/bin/copilot` | GitHub Copilot | Needs the GitHub hosts. Where its login token lives (keychain or file) is unverified. |
| Antigravity | `~/.local/bin/agy` | Google Antigravity | An Electron IDE. Seatbelt on a GUI app and its helper processes is unverified. Last in order. |
| octet | `~/.local/bin/octet`, not installed; a trial ran a pinned v0.10.0 artifact from a scratch directory | Per run | Needs `--safe-mode`: its default effect policy is `UnsafeHost`, which approves every tool call. The profile denies its credential store (`~/.octet/credentials`) and the Keychain, so a contained session takes a key by `--keep-env` or a local provider. Measured 2026-11-07: direct egress refused, the proxy admitting only the provider's hosts, and a read of the gcloud application-default credentials refused with `Operation not permitted`. Verdict and revisit condition in `upstreams.toml`. |

## 5. Order and done-when

1. **Profile and launcher.** Reads are denied by default: system paths and the home toolchains
   (`~/.rustup`, `~/.cargo/bin` and its caches, `~/.bun`) are read-only, the project is
   read-write. The ring and the provider's hosts come from `providers.toml` (`ring`, `hosts`);
   a provider without `ring = "trusted"` runs contained. Checked by
   `macos_sandbox_denies_reads_outside_the_project_and_still_builds`: a sibling read and
   `ls ~` fail, a project write and `cargo test` pass. The Keychain's Mach services are denied.
   `sjel run <harness-bin> --provider <name> --project <dir>` starts it. A harness's own
   directories are opened with `--read <dir>` and `--write <dir>`, and its provider credential
   with `--keep-env <NAME>`, which passes that one variable and no other, with
   `--env <NAME=VALUE>` for a value the caller writes itself, or with
   `--credential <provider>`, which reads `inference-<provider>` from the catalogue `sjel provider`
   reads and hands the child its own variable (`DEEPSEEK_API_KEY`). The value stays in the
   launcher: it is in no command line, shell history or parent environment
   (`a_credential_the_catalogue_cannot_resolve_stops_the_run`). A harness's auth file is never
   opened: pi's holds every provider's key.
   **Measured, pi contained.** `tools/contain/pi` prepares a pi home in the overlay whose
   `models.json` takes the provider's key from the environment, and starts the launcher with
   that home as `HOME`, pi's install readable and `--credential <provider>`. A contained run
   answered a model call (`contained-ok`) through the proxy, with the Keychain, the overlay and
   `~/.ssh` outside its reach. The catalog is the reason: the checkout's own pi settings resolve
   the key by running `security`, which the profile denies by design. Not checked: opencode.
2. **Egress proxy.** The proxy admits the provider's `hosts`, the package registries and
   loopback, and logs each refusal to stderr. Checked live: through the proxy
   `api.deepseek.com` answers, `index.crates.io` answers, `example.com` is refused.3. **Sjel data for `contained`: the demo node only.** The launcher sets `SJEL_STATUS_URL` to
   the demo node, `SJEL_ENV=dev`, and `SJEL_DEMO_TOKEN` from `demo/state/comms-api-secret`.
   The proxy forwards to no loopback address, and the Keychain's Mach services are denied:
   a throwaway item readable by every app is found outside the sandbox and not inside it.
   Not checked end to end: a contained read of the demo node with that token. Open: a contained
   process can still connect to any loopback port directly, so the live node is closed by its
   token alone; denying the live ports in the profile is the second layer.
4. **Copilot CLI**, then **Antigravity**, each with the checks of item 1.
5. **`crates/agent` `project` tier** on the same library. Closes A1.
6. **Provider entries.** `anthropic`, `github-copilot`, `opencode-go` and `antigravity` each get a
   reviewed entry with a ring. The review (`why`, `reviewed_at`) is a human task.
   - **One entry per variant.** A subscription and an API key from the same company have
     different terms. The `openai` entry reviews the API with zero data retention; Codex on a
     ChatGPT login is a subscription, and a subscription cannot get zero data retention. Each
     needs its own entry, for example `openai-chatgpt` and `anthropic-claude`.
   - **A subscription is `trusted` only with training off.** The account toggle is checked by
     a person and recorded in `why` with its date. For OpenAI, the text, voice and video toggles
     are three separate settings. Unverified: whether Claude Code's `/bug` and `/feedback` send a
     transcript outside the training opt-out. Until that is checked, the tracked Claude Code
     settings disable both.
   - **An aggregator lists its models.** OpenCode Go and OpenRouter route to upstreams with
     different retention terms. Their entries list the models allowed at their class. A model not
     listed is refused, the same as an unlisted provider.
   - **Gmail-derived data needs a provider that does not train.** The Google API Services User
     Data Policy (Limited Use) restricts passing Gmail data to models that train on it. Check
     the exact wording before item 7 ships: it decides whether a subscription with training off
     qualifies.
7. **`c2` to the `trusted` ring (C1).** `providers.toml` accepts `c2` as `highest_data_class`;
   `content_item::cloud_admission` admits `c2` as a `trusted_view` derivative to a `c2` provider
   only. The doctrine changes in the same commit: `website/content/architecture/security.md` (Data classes)
   and `shared-data-boundaries.md`. The trusted harnesses keep the overlay (§1, 2026-11-04).
   Done when: a `trusted` agent token reads a `c2` mail with names readable and its sender's
   address replaced, a `pseudonymized` token gets the same mail tokenized, and
   `comms egress-log --audit` counts `c2` sent to a non-`c2` provider as a violation.
8. **Session ratchet.** An agent session records the highest data class it has read. A session
   that has read `c2` cannot continue on a provider whose `highest_data_class` is lower, and its
   transcript cannot be handed to one. Item 7 opens this path, so it ships with item 7 or right
   after it. Done when: a session that read a `c2` mail and then switches from `anthropic` to
   `deepseek` is refused, and the refusal is in the call log.
   A trusted pi run gets no proxy, so a provider switch inside the session is unbounded until
   this ships.
9. **Sjel from the Claude and ChatGPT apps.** `tools/sjel-mcp` is served over the tailnet, with
   an agent token on the `trusted` ring, so the phone apps of both subscriptions can read Sjel
   through `trusted_view`. Whether a consumer subscription may call a remote MCP server this way
   is unverified, and so is whether the ChatGPT app on iOS accepts one. Done when: the Claude
   app on the phone answers a question from a `c2` mail, the call log shows `trusted`, and a
   `c3` object is withheld.

## 6. Open decisions

| # | Question | State |
|---|---|---|
| C1 | `trusted` providers read raw overlay data, c2 included. `shared-data-boundaries.md` says c2 stays local-only, and the success measure in `PLAN.md` says no raw data about other people leaves the machine. Which one changes? | Decided 2026-11-03: both change. `c2` reaches the `trusted` ring through `trusted_view` with full names. Raw files do not: every harness loses the overlay `data/`. `c2` today also holds health and tax mail (`libs/content-item/src/lib.rs:601`), so that mail reaches Claude and GPT too. This was chosen over splitting `c2`. Work: §5 item 7. Amended 2026-11-04: the trusted ring keeps the overlay, and every other provider gets the demo node only (§1). |
| C2 | Where does the ring live: a new `ring` field in `providers.toml`, or derived from `highest_data_class` (c2 or above is `trusted`)? | Decided 2026-11-04: an explicit `ring = "trusted"` field (`crates/containment/src/ring.rs`). Deriving it from `c2` would make every `c2` provider trusted without a separate review. `openai` and `anthropic` set it; every other entry in `providers.toml` runs contained. |
| C3 | May a contained harness reach the macOS keychain for its own login? `mach-lookup` is unfiltered today (§5 item 1). Denying it is safer and may break the Copilot and Antigravity logins. | Open, measure in item 4. |
| C4 | Does Sjel ever call a consumer subscription itself, for example `claude -p` from a scheduled job? A subscription reaches Sjel only when the user starts it, through MCP (item 9), and Sjel's own model calls use API keys. This avoids running automated jobs on a consumer plan, which the terms may not allow (unverified). | Decided 2026-11-03: no scheduled or background call. Narrowed the same day: a foreground command the user runs may start a harness, as `tools/intake review --reviewer claude` does. |
| C5 | The data-class cap of a contained agent token (§5 item 3). A row with no `data_class` under a `c0` cap: withheld or passed pseudonymized? A list with rows above the cap: those rows withheld, or the whole response refused? | Decided 2026-11-04: an unclassified row is withheld, rows above the cap are dropped, and a single item above the cap or without a class is refused. Built (§5 item 3). |
| C6 | Does the profile admit each ancestor of a granted path, so a reader that walks with `openat` works? | Open. Measured 2026-11-08 (§2): the walk needs `file-read-data` on each ancestor, since `file-read-metadata` is not enough. Admitting them leaves `ls ~` listing names and never file contents. |
