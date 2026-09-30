# pihole

Reusable Pi-hole runtime contract for a private network. Sjel owns the pinned image, host-network
requirements, capability permissions, lifecycle integration, public environment template, and
backup contract. The active overlay owns DNS records, clients, query data, credentials, topology,
and deployment evidence.

## Runtime contract

Pi-hole uses host networking so client source addresses survive and DNS can bind the host service
port. It receives `NET_ADMIN`, not unrestricted privileged access. The selected overlay decides
whether this capability is appropriate for its host and supplies all network-specific values.

```sh
tools/service-runner.sh start pihole
tools/service-runner.sh status pihole
tools/service-runner.sh stop pihole
tools/service-runner.sh resume pihole
```

## Configuration and data

- `pihole.env.example` contains neutral public defaults and placeholders.
- `<overlay>/config/pihole.env` contains deployment values and secret references.
- `<overlay>/data/pihole/` contains private runtime state and query history.
- `Packs/home-automation/skills/pihole/` provides the generic API workflow.

The public backup contract reads the declared container path and sends it to the overlay-selected
`backup-target`. Target coordinates and restore evidence never belong in Sjel.

## Attribution

The official Pi-hole image is adopted and pinned in `upstreams.toml`. Image findings and exception
expiry remain public supply-chain concerns; live rollout evidence belongs to the owning overlay.
