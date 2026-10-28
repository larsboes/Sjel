# obsidian-access

Pi-native, policy-aware tools over `obsidian-local-rest-api`. The Obsidian plugin remains the vault transport. This package decides what may enter the active model's context. The integration is live-tested against plugin 5.3.1.

The package exposes the vault-writing contract configured for the machine, governed note and Base reads, draft validation, discovery, backlinks, active-file state, properties, tags, and Markdown mutations. It deliberately does not configure or proxy the plugin's unrestricted MCP endpoint.

## Data boundary

Sjel's C0–C3 class remains authoritative. C3 never enters a prompt. `agent_access` is an optional, stricter frontmatter constraint:

```yaml
agent_access: internal # or confidential
```

An absent field adds no restriction unless the note carries the vault's older `sensitivity:` policy: `internal` maps to internal agent access and `restricted` maps to confidential. Private `[[paths]]` rules tighten access for boundaries such as `_Internal/` and for non-Markdown objects that cannot carry frontmatter. `agent_access` and path rules may tighten existing boundaries but cannot weaken them. Invalid frontmatter and unknown policy values fail closed. The package calls `vault classify`, which delegates C0–C3 decisions to `content_item::DataClass::classify_vault_note`. If the binary is unavailable, an undeclared object uses a conservative C2 fallback rather than copying the folder rules into TypeScript.

Listings and searches classify each candidate before returning its filename or snippet. Markdown candidates include their frontmatter restrictions; other file types use their path classification. A denied object therefore does not become visible through discovery.

## Writing contract

The writing contract is optional and machine-specific. The overlay's `config/knowledge.toml` names it with `contract_path`, a vault-relative Markdown path, next to the optional `profile`:

```toml
contract_path = "Meta/Vault-Contract.md"
```

`obsidian_contract` reads that note through Obsidian's API. While a contract is configured, every mutation tool refuses to run until the contract has been loaded in that session; resumed sessions accept the prior load only while its SHA-256 digest still matches the current vault note. A configured contract that is missing, empty, larger than 50,000 bytes, or unreadable fails closed, and so does an invalid `knowledge.toml`. Without an overlay, a `knowledge.toml`, or a `contract_path`, `obsidian_contract` reports that no contract is configured and mutations are not gated on one.

## Validation and Bases

`obsidian_note_validate` checks strict YAML frontmatter and policy fields on a Markdown draft. `obsidian_base_validate` checks strict YAML, top-level section shapes, required view fields, string expressions, and undefined `formula.*` references. Neither tool writes to the vault.

`obsidian_base_read` admits a `.base` file through the same model and path policy used by discovery, validates it, and returns its YAML with diagnostics. Base mutations are deliberately absent: plugin 5.3.1 does not provide the markdown-patch version precondition used for safe note edits, so whole-file Base writes would risk overwriting concurrent Obsidian changes.

## Mutations

The mutation tools operate on `.md` files only. Create stages content in `Resources/Inbox/` under a UUID filename and moves it into place without overwrite; a destination collision removes the staging file and fails the tool. Append and whole-note replacement carry the document-map version as an `ifMatch` precondition. Targeted patches do the same, so a concurrent edit returns HTTP 412 instead of modifying the newer note.

Whole-note replacement, move, and delete always ask for confirmation. A targeted replacement or policy-field edit also asks. Copy requires confirmation only when it would overwrite a destination. A headless Pi session cannot approve those operations. The package refuses destination classifications the active model cannot access, and a mutation that makes a note inaccessible returns no resulting content or protected metadata.

## Session boundary and audit

The session stores the highest C0–C2 and `agent_access` clearance returned by Obsidian. Switching to a weaker model blocks the next turn. C2 or confidential clearance removes the configured egress tools and also blocks their calls at the tool hook; unrestricted shell networking still requires an Agentbox profile for a hard boundary.

The default private audit log is `$SJEL_OVERLAY_ROOT/logs/obsidian-access.jsonl`. Rows contain the time, model, operation, decision, classification, and a SHA-256 path digest. They contain no note content, search query, API key, or raw path.

## Private configuration

Copy `obsidian-access.toml.example` to the active overlay as `config/obsidian-access.toml`, then replace the example model patterns with reviewed `provider/model` identifiers. `[[paths]]` accepts vault-relative, case-insensitive globs and an `internal` or `confidential` access floor. Model rules name their processing boundary as `local`, `trusted-service`, or `external`. C2 requires the separate, explicit `c2_approved = true`; proximity to localhost does not imply approval. With no file, unmatched models receive the default C0-only policy.

The REST credential comes from `OBSIDIAN_API_KEY`. The extension never writes it into a tool argument, result, prompt, or log. This setup treats the local Pi process as trusted, so the key remains inherited by Pi; use a broker and process isolation if that trust boundary changes.

## Development

```sh
cargo build --release -p vault --bin vault
cd Packs/knowledge/pi-packages/obsidian-access
bun install --frozen-lockfile --ignore-scripts
bun run check
```

Pi loads `src/index.ts` directly. There is no build output.
