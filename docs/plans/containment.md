# Agent containment

Additional harness support follows Priority 3. Existing containment remains in force. The
[containment crate](../../crates/containment/README.md) owns runtime mechanics; provider entries
own explicit ring, hosts and data-class admission.

## 1. The rule

The provider, not harness branding, selects the ring. Unknown/unreviewed providers are contained.
Trusted agents read personal data only through the Sjel gate; c3 stays withheld and a response
must not pair a real name with other facts about that person. Direct overlay data/secrets and
home credentials stay closed. A command classifier is a second layer, not an OS sandbox.

Contained runs write only the selected project, read admitted toolchains and use provider/registry
proxy egress plus the demo node. Ancestor directory names are admitted for safe component-by-component
path walking; ungranted contents are not. The launcher passes only selected credentials and a
per-run CA bundle; values never enter command lines or reports. Child processes inherit the profile.

Pi's trusted-provider debug escape requires an overlay audit record. Other harnesses have no escape.
A provider change cannot expand launch-time reach. Subscription/API variants require separate
human review; scheduled or background subscription calls remain excluded.

## Open

- [Accept the installed harness panel](../tasks/containment-panel.md).
- [Verify Codex MCP admission](../tasks/containment-codex-mcp.md).
- [Accept a contained OpenCode run](../tasks/containment-opencode.md).
- [Close live-node loopback reach](../tasks/containment-demo.md).
- [Accept reviewed additional harnesses](../tasks/containment-harnesses.md).
- [Share containment with project sessions](../tasks/containment-project-tier.md).
- [Review exact provider variants](../tasks/containment-provider-review.md).
- [Prevent session class downgrade](../tasks/containment-ratchet.md).
- [Review consumer-app MCP access](../tasks/containment-phone-apps.md).
- [Resolve native Claude TLS refusal](../tasks/containment-claude-ca.md).
- [Review zero-retention routing](../tasks/containment-openrouter.md).
