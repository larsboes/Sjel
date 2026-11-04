# Agent containment by provider

A coding agent gets the reach its model provider has earned. An agent on a contained provider
works in one project directory with no route to the user's data, other checkouts, the overlay or
credentials. Inside that directory it runs without approval cards, because the operating system
stops everything outside it.

Priority 0 in [`PLAN.md`](../../PLAN.md) (decided 2026-10-31). It also closes decision A1 in
[`agent.md`](agent.md#10-decisions): the macOS sandbox profile for the `project` tier.

## 1. The rule

Decided 2026-10-31.

- **The provider sets the ring, not the harness.** Most harnesses use one provider. A harness
  that can call several (pi, opencode) gets the ring of the provider it is started with.
- **Two rings.**

| Ring | Providers | Filesystem | Network | Sjel data |
|---|---|---|---|---|
| `trusted` | No training on user data, reviewed retention terms. Claude and GPT, by subscription or API. | As today, except the overlay `data/` directory | As today | Only through the agent gate, as `trusted_view`: `c0` to `c2` readable with full names, contact details and locations replaced, `c3` withheld. Development uses a synthetic store. |
| `contained` | Every other provider: DeepSeek, OpenCode Go, GitHub Copilot, Antigravity, and any provider not listed | The project directory read-write. Toolchains and the harness's own config directory read-only. Nothing else. | The provider's endpoints, package registries and the Sjel gateway. Nothing else. | Only through the agent gate, pseudonymized, capped at the provider's `highest_data_class` |

- **A provider not listed in `providers.toml` is `contained` with no Sjel data.** This follows the
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
Whether opencode or the Copilot CLI has a sandbox of its own is unverified.

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
   credentials). Set `HTTPS_PROXY` and the agent token for the provider's ring.
5. `exec` the harness under `sandbox-exec`. Child processes inherit the profile.

Placement: the profile generator and the proxy go in one library that `sjel run` and
`capabilities/agent` share, so the agent's `project` tier gets the same boundary (A1). Rust,
no npm (decided 2026-10-31). The proxy reuses the workspace's existing HTTP stack.

## 4. Harnesses

| Harness | Binary | Provider | Notes |
|---|---|---|---|
| pi | `~/.pi` | Per run (DeepSeek, OpenCode Go, ...) | The proxy admits only the provider named at launch, so a provider switch inside the session cannot widen it. To do: route DeepSeek through OpenRouter with zero data retention enforced per request instead of `api.deepseek.com`. The `openrouter` entry in `providers.toml` then needs a new review for `c1`. |
| opencode | `~/.opencode/bin/opencode` | Per run (OpenCode Go, ...) | Same as pi. |
| Copilot CLI | `~/.local/bin/copilot` | GitHub Copilot | Needs the GitHub hosts. Where its login token lives (keychain or file) is unverified. |
| Antigravity | `~/.local/bin/agy` | Google Antigravity | An Electron IDE. Seatbelt on a GUI app and its helper processes is unverified. Last in order. |

## 5. Order and done-when

1. **Profile and launcher.** Reads are denied by default: system paths and the home toolchains
   (`~/.rustup`, `~/.cargo/bin` and its caches, `~/.bun`) are read-only, the project is
   read-write. The ring and the provider's hosts come from `providers.toml` (`ring`, `hosts`);
   a provider without `ring = "trusted"` runs contained. Checked by
   `macos_sandbox_denies_reads_outside_the_project_and_still_builds`: a sibling read and
   `ls ~` fail, a project write and `cargo test` pass. Not checked: pi and opencode
   themselves under the profile, and their config directories (`--read`, `--write` are not
   built). Open: `mach-lookup` is allowed without a filter, so a contained process can ask
   `securityd` for a Keychain item. Whether the Keychain ACL then prompts is unverified.
2. **Egress proxy.** The proxy admits the provider's `hosts`, the package registries and
   loopback, and logs each refusal to stderr. Checked live: through the proxy
   `api.deepseek.com` answers, `index.crates.io` answers, `example.com` is refused.
3. **Sjel data for `contained`.** The gate applies a per-token data-class cap
   (`sjel agent cap <name> <c0|c1|c2|none>`, `cap_view` in `crates/pseudonymize/src/view.rs`):
   rows above the cap or without a class are removed and counted in `x-sjel-withheld`, and a
   single item above the cap or without a class is refused. Checked by
   `a_capped_token_reads_only_rows_at_or_below_its_cap`. Open: the launcher passes no agent
   token yet. Owner steps: `sjel agent enroll contained-<provider>`, then
   `sjel agent cap contained-<provider> <the provider's highest_data_class>`. Then the launcher
   hands that token to the run. Done when a contained run reads a capability through the gate
   with that token, and a c3 and an unclassified row are withheld.
4. **Copilot CLI**, then **Antigravity**, each with the checks of item 1.
5. **`capabilities/agent` `project` tier** on the same library. Closes A1.
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
   and `shared-data-boundaries.md`. The overlay `data/` is denied in the Claude Code and Codex
   sandbox settings, and the demo node replaces it for debugging (`sjel demo up`, [`gpui.md`](gpui.md#10-dev-and-prod)). Done when: a
   `trusted` agent token reads a `c2` mail with names readable and its sender's address replaced,
   a `pseudonymized` token gets the same mail tokenized, `cat <overlay>/data/...` fails in a
   Claude Code session, and `comms egress-log --audit` counts `c2` sent to a non-`c2` provider
   as a violation.
8. **Session ratchet.** An agent session records the highest data class it has read. A session
   that has read `c2` cannot continue on a provider whose `highest_data_class` is lower, and its
   transcript cannot be handed to one. Item 7 opens this path, so it ships with item 7 or right
   after it. Done when: a session that read a `c2` mail and then switches from `anthropic` to
   `deepseek` is refused, and the refusal is in the call log.
9. **Sjel from the Claude and ChatGPT apps.** `tools/sjel-mcp` is served over the tailnet, with
   an agent token on the `trusted` ring, so the phone apps of both subscriptions can read Sjel
   through `trusted_view`. Whether a consumer subscription may call a remote MCP server this way
   is unverified, and so is whether the ChatGPT app on iOS accepts one. Done when: the Claude
   app on the phone answers a question from a `c2` mail, the call log shows `trusted`, and a
   `c3` object is withheld.

## 6. Open decisions

| # | Question | State |
|---|---|---|
| C1 | `trusted` providers read raw overlay data, c2 included. `shared-data-boundaries.md` says c2 stays local-only, and the success measure in `PLAN.md` says no raw data about other people leaves the machine. Which one changes? | Decided 2026-11-03: both change. `c2` reaches the `trusted` ring through `trusted_view` with full names. Raw files do not: every harness loses the overlay `data/`. `c2` today also holds health and tax mail (`libs/content-item/src/lib.rs:601`), so that mail reaches Claude and GPT too. This was chosen over splitting `c2`. Work: §5 item 7. |
| C2 | Where does the ring live: a new `ring` field in `providers.toml`, or derived from `highest_data_class` (c2 or above is `trusted`)? | Decided 2026-11-04: an explicit `ring = "trusted"` field (`crates/containment/src/ring.rs`). Deriving it from `c2` would make every `c2` provider trusted without a separate review. No entry sets it yet, so every provider runs contained. |
| C3 | May a contained harness reach the macOS keychain for its own login? `mach-lookup` is unfiltered today (§5 item 1). Denying it is safer and may break the Copilot and Antigravity logins. | Open, measure in item 4. |
| C4 | Does Sjel ever call a consumer subscription itself, for example `claude -p` from a scheduled job? A subscription reaches Sjel only when the user starts it, through MCP (item 9), and Sjel's own model calls use API keys. This avoids running automated jobs on a consumer plan, which the terms may not allow (unverified). | Decided 2026-11-03: no scheduled or background call. Narrowed the same day: a foreground command the user runs may start a harness, as `tools/intake review --reviewer claude` does. |
| C5 | The data-class cap of a contained agent token (§5 item 3). A row with no `data_class` under a `c0` cap: withheld or passed pseudonymized? A list with rows above the cap: those rows withheld, or the whole response refused? | Decided 2026-11-04: an unclassified row is withheld, rows above the cap are dropped, and a single item above the cap or without a class is refused. Built (§5 item 3). |
