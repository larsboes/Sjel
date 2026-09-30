# libs/pseudonymize

Reversible bidirectional session tokenization for cloud evaluators and agents (PRD §6.2c, Q112, Slice P1).

## Why this exists

Destructive one-way redaction (`[person]`, `[link]`) was built for static feed and mail triage
(`capabilities/comms/src/cloud_derivative.rs`). It protects privacy by erasing identity.
That erasure is fatal for interactive assistants and cloud evaluators: if a trip query scrubs
"Karlsruhe" and "Munich" both to `[place]`, an external routing model or Jev cannot reason over
origin vs. destination, and Sjel cannot re-hydrate the resulting travel legs upon return.

This library replaces destructive masking with **bidirectional session tokenization**:
- Personal and operational entities are mapped deterministically to typed tokens: `Lars` → `<TRAVELER_01>`,
  `Karlsruhe Hbf` → `<PLACE_01>`, `München Hbf` → `<PLACE_02>`.
- The external evaluator (Jev / cloud LLM) reasons over structure, constraints, and tokens without
  ever receiving real personal identifiers.
- On completion return, Sjel re-hydrates tokens locally (`<PLACE_01>` → `Karlsruhe Hbf`) before
  storing in SQLite or rendering UI cards.

## What it does

The ladder is PRD §6.2's, applied in `session.rs` (`tokenize_text`):

1. **Rung 1, shapes (`pattern.rs`).** Word-level predicates for links, email addresses, IBANs,
   phone numbers, token-like secrets and any word with six or more digits. Hand-written
   predicates, not regular expressions and not a port of Presidio's catalog. They are the same
   functions the destructive path in `capabilities/comms/src/cloud_derivative.rs` calls, so the
   two transformations cannot drift apart.
2. **Rung 0, known names (`registry.rs`).** An Aho-Corasick dictionary over a folded copy of
   the text: Unicode lowercase plus `ß`→`ss`, `ä`→`ae`, `ö`→`oe`, `ü`→`ue`. A hyphenated compound
   (`Anna-Lena`) becomes one entity; a person followed by a genitive `s` (`Annas`) keeps the
   person's token and leaves the `s` in place. comms builds the dictionary from its people
   registry (`people_registry::entity_registry`).
3. **Rung 2, cues (`pattern.rs`).** A capitalised word or a login handle after a salutation or
   a self-introduction, a person named as being from an organisation, and the word after any
   person.

`PseudonymizerSession` holds the symbol table:

- Tokens are keyed on the exact spelling, so rehydration restores the original case.
- Token-shaped text already in the source (`<EMAIL_01>`) is escaped to a `<LITERAL_nn>` token,
  so rehydration cannot turn it into a real value.
- `findings()` counts occurrences since the session began or since the last
  `take_findings()`. A caller that needs a per-call receipt (PRD Q9b) or a reproducible
  document hash uses one fresh session per call.
- `Debug` prints counts only. The maps hold the personal values.

Not covered: decomposed Unicode umlauts (`u` + U+0308), postal addresses, and names that no
rung recognises.

## Keyed sessions and the agent view

Two additions serve agents that read Sjel through its APIs (ISA F9).

- **Keyed tokens (`keyed.rs`).** `PseudonymizerSession::keyed(key)` issues
  `<TRAVELER_k3x9qa>` instead of `<TRAVELER_01>`. The suffix is HMAC-SHA256 over the entity
  type and the exact value, so two processes that hold one key give one value the same token
  without shared state. `keyed::session_key` derives the key from a machine secret and a
  session id. Without that secret, a party that sees a token cannot test a guessed name
  against it.
- **The agent view (`view.rs`).** `view::agent_view` rewrites a JSON response by field name.
  Ids, timestamps, enums and classes stay verbatim, so follow-up calls still work. Sender and
  recipient fields become one identity token each. Other strings go through the ladder. Any
  object with `data_class` `c3` is removed and counted.

`libs/sjel-server/src/agent.rs` applies both at the gate to every response for the agent
token, on each capability that calls `InboundAuth::admit_agents`.

## Usage

```rust
use sjel_pseudonymize::{Pseudonymizer, EntityRegistry};

let registry = EntityRegistry::builder()
    .add_people(["Lars", "Anna"])
    .add_places(["Karlsruhe Hbf", "München Hbf"])
    .build();

let engine = Pseudonymizer::new(registry);
let mut session = engine.new_session();

// 1. Tokenize request
let prompt = "Can Lars meet Anna at Karlsruhe Hbf before going to München Hbf?";
let tokenized = engine.tokenize_text(&mut session, prompt);
// -> "Can <TRAVELER_01> meet <TRAVELER_02> at <PLACE_01> before going to <PLACE_02>?"

// 2. Pass tokenized text or JSON to Jev / remote model...
// 3. Re-hydrate response locally
let rehydrated = engine.rehydrate_text(&session, &tokenized);
assert_eq!(rehydrated, prompt);
```
