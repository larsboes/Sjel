# host-firewall

A default-deny host firewall for a Linux node, rendered from the active overlay's declaration.

Sjel owns the posture, the rule shape, the ordering and the lifecycle. The overlay owns four
facts about one machine: which interface faces the network, which interfaces are trusted,
which sources may connect, and which ports they may reach. Nothing here names a real address.
`host-firewall.toml.example` uses RFC 5737 and RFC 3849 documentation ranges, which are
unroutable by design.

## Why it exists here rather than in an overlay

Before this, a consumer had to assemble both the rules and the deployment logic privately.
Almost none of that is specific to a machine. The established/related accept, the loopback
rule, the ICMPv6 subset IPv6 cannot function without, and the order that makes all three
correct are the same on every node. Only the four inputs differ. Copying the other 90% into
each overlay is how one node ends up with an IPv6 hole the other one closed.

## Use

```sh
host-firewall render     # the complete effective ruleset, to stdout. Changes nothing.
host-firewall check      # validate the inputs, and the ruleset itself when nft is present
sudo host-firewall apply # install it behind a countdown that rolls back on silence
host-firewall confirm    # cancel that countdown, once you have verified you are still connected
sudo host-firewall rollback
host-firewall status
```

Run `render` before `apply`, always. The ruleset is short enough to read, and reading it is
cheaper than recovering from it.

## Lockout recovery

A firewall is the one capability that can sever the hand applying it. So `apply` never
installs permanently in one step:

1. the current ruleset is saved to `<overlay>/data/host-firewall/previous.nft`
2. the new ruleset is loaded
3. a detached countdown restores the saved one after `confirm_seconds` (default 60)
4. `host-firewall confirm` cancels it

Step 3 detaches deliberately. The case it exists for is the one where the shell that started
it is already gone. That makes the failure mode of a wrong rule "the connection comes back in
a minute" rather than "drive to the machine".

If you are reading this because you *are* locked out: wait 60 seconds and reconnect. If a
`confirm` already ran, physical or out-of-band access is the only path left. That is why
`confirm` belongs after you have verified the connection, never in the same command.

## What this does not do

It does not filter egress. `output` is `policy accept`, deliberately. A default-deny outbound
policy on a host running containers and package managers becomes a maintenance burden that
gets switched off within a week, and a rule everyone switches off is worse than no rule.

It does not apply itself. There is no autostart and no watchdog. Loading a firewall is an
explicit, root, operator-run action.

It does not know your topology. There is no default interface and no default port set, because
a guess here is a lockout.

## Verification

`tools/host-firewall.test.sh` covers the render layer: default-deny posture, established
traffic, loopback, the required ICMP/ICMPv6 subset, rule ordering, input substitution, and the
refusals. Applying to a real host and proving its effective policy remain the consuming
overlay's work. The tests here prove the ruleset is right, not that it is loaded.
