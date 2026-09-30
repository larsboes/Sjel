# macmon

What this machine's silicon is actually doing — temperature, power draw, CPU and GPU
utilisation, memory — read without sudo.

Sjel does not build this one. [macmon](https://github.com/vladkens/macmon) is an adopted
Apple Silicon performance monitor (`upstreams.toml [macmon]`, MIT, installed with
`cargo install`); this capability is the manifest that says how this machine runs it,
nothing more.

It has two unrelated uses, and only one of them is this manifest. `tools/sysmon power`
execs the binary as an interactive TUI on demand and needs no server. This capability is
the other one: `macmon serve`, the long-running HTTP endpoint that answers JSON at
`/json` and Prometheus at `/metrics`, whose only consumer is the dashboard's **/systems**
page.

## Why it is a capability and not a LaunchAgent

It used to be a LaunchAgent nobody generated. The unit was written by hand, exempted from
`tools/doctor`'s orphan check through the `sidecars` seam in `dashboard/service.toml`
(Axon#65), and therefore versioned nowhere — its port and sampling interval lived in one
untracked file under `~/Library/LaunchAgents`. Rebuild the machine and it comes back
wrong, or not at all, with doctor still green: the exemption that stopped the false
positive also removed the only check that would have noticed.

A manifest costs less than the exception did. Persistence is now rendered by
`tools/service-runner.sh install-persistence` like every other unit, the port reaches
both the process and the dev-server proxy from one field, and the orphan check covers it
by the ordinary rule instead of skipping it by name.

## macOS only

macmon reads Apple Silicon's own power counters, so it is enabled on this Mac and would
be meaningless on the family Pi. That is a per-machine fact and lives where those live —
`capabilities = [...]` in the overlay's `config/machine.toml` — not in a condition here.
