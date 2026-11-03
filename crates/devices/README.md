# devices

Registry for device pairing, public keys, signed-request replay protection and revocation. It owns device identity, not capability data or relay transport.

## Why this shape: keys establish device identity

A pairing claim registers an Ed25519 public key; the private key remains on the device. Signed requests bind the method, path, body digest, timestamp and nonce. The registry checks revocation and replay state before admitting a request, so network reachability alone does not establish device identity.

Unregistered devices may use only the bounded pairing routes needed to claim or request access. Other device requests require a valid signature.

## Run

The service starts with the deployment through `tools/service-runner.sh`. The live HTTP contract is available at `GET /routes`.

## Considered and declined

- **No capability data or relay transport.** Device enrollment and authentication stay separate from the services that own user data and from any network relay.
