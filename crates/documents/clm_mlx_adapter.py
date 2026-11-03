#!/usr/bin/env python3
"""Loopback System One adapter for the local CLM MLX port.

The service is intentionally bound to 127.0.0.1, keeps one model instance in memory,
accepts one bounded JSON request per call, and never writes request content to disk or logs.
"""
from __future__ import annotations

import argparse
import hmac
import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

MAX_BODY_BYTES = 128 * 1024
MAX_STATE_CHARS = 64_000
MAX_TOKENS = 2_048
MAX_QUESTIONS = 16
MAX_OPTIONS = 12
MODEL_ID = "mlx-community/CLM-v0.1-8B-MLX-4bit"
MODEL_REVISION = "489b421bafc007e3ae409528d77006d26d362689"


def validate(state: object, questions: object, engine: object) -> None:
    if not isinstance(questions, dict) or not 1 <= len(questions) <= MAX_QUESTIONS:
        raise ValueError("invalid question set")
    if len(json.dumps(state, ensure_ascii=False)) > MAX_STATE_CHARS:
        raise ValueError("state too large")
    for question in questions.values():
        if not isinstance(question, dict):
            raise ValueError("invalid question")
        kind = question.get("type")
        criteria = question.get("criteria")
        instructions = question.get("instructions", "")
        if not isinstance(instructions, str) or len(instructions) > 500:
            raise ValueError("invalid instructions")
        if kind == "choice":
            if not isinstance(criteria, dict) or not 2 <= len(criteria) <= MAX_OPTIONS:
                raise ValueError("invalid choice options")
        elif kind == "score":
            if not isinstance(criteria, list) or not 2 <= len(criteria) <= MAX_OPTIONS:
                raise ValueError("invalid score options")
        elif kind != "noul":
            raise ValueError("invalid question type")
    from clm_mlx.schema import build_pairs

    for state_text, _, candidate_texts in build_pairs(state, questions).values():
        if len(engine.encoder.ids(state_text)) > MAX_TOKENS:
            raise ValueError("input exceeds CLM's 2,048-token limit")
        if any(len(engine.encoder.ids(candidate)) > MAX_TOKENS for candidate in candidate_texts):
            raise ValueError("candidate exceeds CLM's 2,048-token limit")


def make_handler(engine: object, model_name: str, api_key: str | None):
    from clm_mlx.schema import build_pairs

    engine_lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        server_version = "SjelCLM/1"
        sys_version = ""

        def log_message(self, _format: str, *_args: object) -> None:
            # Request paths, headers and content are intentionally not logged.
            return

        def _send(self, status: int, body: dict) -> None:
            raw = json.dumps(body, ensure_ascii=False, separators=(",", ":")).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(raw)))
            self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff")
            self.end_headers()
            self.wfile.write(raw)

        def _local_origin(self) -> bool:
            host = self.headers.get("Host", "").split(":", 1)[0].strip("[]").lower()
            if host not in {"127.0.0.1", "localhost"}:
                return False
            # Browser requests are not part of the contract. Rejecting every Origin prevents
            # a page on the operator's browser from using this unauthenticated loopback port.
            return not self.headers.get("Origin")

        def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
            if not self._local_origin():
                self._send(403, {"error": "loopback origin required"})
                return
            if self.path != "/health":
                self._send(404, {"error": "not found"})
                return
            self._send(200, {
                "ok": True,
                "embedder": True,
                "mock": False,
                "models": [model_name],
                "model_id": MODEL_ID,
                "revision": MODEL_REVISION,
            })

        def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
            if not self._local_origin():
                self._send(403, {"error": "loopback origin required"})
                return
            if self.path != "/v1/systemone":
                self._send(404, {"error": "not found"})
                return
            if api_key is not None:
                supplied = self.headers.get("Authorization", "")
                if not hmac.compare_digest(supplied, f"Bearer {api_key}"):
                    self._send(401, {"error": "unauthorized"})
                    return
            try:
                length = int(self.headers.get("Content-Length", "-1"))
                if not 0 <= length <= MAX_BODY_BYTES:
                    raise ValueError("invalid body size")
                payload = json.loads(self.rfile.read(length))
                if not isinstance(payload, dict) or payload.get("model") != model_name:
                    raise ValueError("unknown model")
                state, questions = payload["state"], payload["questions"]
                validate(state, questions, engine)
                with engine_lock:
                    result = engine.answer(state, questions)
                self._send(200, {
                    "model": model_name,
                    "revision": MODEL_REVISION,
                    "answers": result["answers"],
                    "usage": result["usage"],
                })
            except (ValueError, KeyError, TypeError, json.JSONDecodeError):
                self._send(400, {"error": "invalid or oversized classification request"})
            except Exception:  # unslop-ignore: sanitize unexpected model errors at this request boundary.
                # Do not return a traceback or echo the document text to the caller.
                self._send(502, {"error": "local CLM inference failed"})

    return Handler


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=int(os.environ.get("SJEL_CLM_PORT", "8700")))
    args = parser.parse_args()
    model_dir = Path(os.environ.get("SJEL_DOCUMENTS_CLM_MODEL_DIR", ""))
    if (
        not model_dir.is_absolute()
        or not (model_dir / "encoder" / "model.safetensors").is_file()
        or not (model_dir / "heads" / "CLM_v0.1-8B.safetensors").is_file()
    ):
        raise SystemExit("Set SJEL_DOCUMENTS_CLM_MODEL_DIR to the installed local MLX model directory")

    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["TRANSFORMERS_OFFLINE"] = "1"
    os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
    sys.path.insert(0, str(model_dir))
    from clm_mlx.engine import Engine

    print("Loading local CLM MLX model…", flush=True)
    engine = Engine(str(model_dir / "encoder"), str(model_dir / "heads"), truncation="tail")
    model_name = os.environ.get("SJEL_SYSTEMONE_MODEL", "clm-latest")
    api_key = os.environ.get("CLM_API_KEY") or None
    server = ThreadingHTTPServer(("127.0.0.1", args.port), make_handler(engine, model_name, api_key))
    server.daemon_threads = True
    print(f"Sjel CLM adapter listening on 127.0.0.1:{args.port} ({model_name})", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()
