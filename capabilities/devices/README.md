# devices

The deployment's device registry. It owns one-time pairing challenges, registered device public
keys, signed-request replay state and revocation state. It does not own capability data or relay
transport.

## Contract

- `POST /api/pairing/challenges` creates a ten-minute challenge. The plaintext code is returned
  once; only its SHA-256 digest is stored.
- `POST /api/pairing/claims` consumes a challenge and registers one Ed25519 public key. The private
  key is never sent to Axon.
- `GET /api/devices` lists active and revoked devices.
- `POST /api/devices/:id/revoke` revokes one active device.
- `GET /api/devices/me` requires `axon-device-auth/v1` headers and returns the authenticated device.

Signed requests use `X-Axon-Device-Id`, `X-Axon-Timestamp`, `X-Axon-Nonce` and
`X-Axon-Signature`. The Ed25519 signature covers the protocol version, device id, timestamp,
nonce, uppercase method, capability path and SHA-256 body digest. Timestamps must be within five
minutes of the node clock. Accepted nonces are retained for ten minutes and are unique per device;
revocation is checked again at the nonce commit.

The Axon status shell also admits a request on a valid device signature (PRD Q119: the device
key, not the network, is the trust root). Its gate (`libs/sjel-server/src/auth.rs`,
`with_device_verifier`) checks every request that carries `X-Axon-Signature` against this registry
through `DevicesStore::authenticate_scoped`, before it reaches any capability. A valid signature
admits the request and the shell sends the deployment token upstream; an invalid one is refused
with `401` and never falls through to the tailnet or token rules. The shell consumes the nonce in
its own `shell` scope, so the same request is not a replay when it reaches `/api/devices/me`.

The service is reached through the Axon status shell at `/devices/api/...`. It remains behind the
existing deployment inbound gate and origin guard. Pairing and operator registry routes remain
protected by that deployment gate; device data routes use the signed device identity as well.

The store uses the shared Axon SQLite file under the `devices_` prefix and is covered by the service
backup contract. Challenge rows are deliberately retained after use so replay attempts can be
reported as "already used" rather than looking like missing state.
