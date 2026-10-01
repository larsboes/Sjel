# Assistant

A local-only OpenAI-compatible chat client for the dashboard's `local` model rung. The service
resolves the `assistant` role from the active overlay's `config/inference.json`, accepts only a
loopback backend, bounds input and output, and returns `503` when the role is absent or not ready.
It does not send prompts to cloud providers.

## Configure the role

Add a dedicated role to the overlay's `config/inference.json`; do not repurpose
`summarization`:

```json
"assistant": {
  "backend": "ollama",
  "model": "qwen3:8b",
  "max_input_tokens": 8192
}
```

The backend must resolve to a loopback URL. `max_input_tokens` is required; the service enforces
its value as a conservative UTF-8 byte ceiling, and the dashboard uses that ceiling when planning
fits. The shared example is in
[`libs/inference/inference.config.example.json`](../../libs/inference/inference.config.example.json).

## Routes

- `GET /health`: liveness only.
- `GET /ready`: reports whether the local role is configured; does not return credentials.
- `POST /api/generate`: `{ "prompt": "...", "instructions": "...", "max_tokens": 512 }`.

The service binds loopback through `sjel_server::serve_local`. Protected requests require the
shared inbound token. The shell injects that token on its server-to-server proxy hop; it is never
sent to browser JavaScript. On iOS, the signed native bridge reaches `/assistant/` through the
same shell proxy.

The configured role's model appears in the readiness response. Generated assistant messages
identify the `Local assistant` rung. Cloud wiring is deliberately deferred until an assistant-
compatible provider contract and per-send approval flow exist.
