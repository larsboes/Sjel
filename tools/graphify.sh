#!/bin/bash
# graphify.sh — rebuild the local code-dependency graph via graphify (upstreams.toml
# [graphify]). Output lands in graphify-out/ (git-ignored -- see
# the header above for why: graphify's
# node IDs are slugified from the absolute scan path, so its output is inherently
# machine-specific and not something ARCHITECTURE.md embeds directly).
#
# oMLX is the local default. Its API key is read at call time from the same
# ~/.omlx/settings.json file the server owns; it is exported only to graphify's child process
# and never printed. tools/setup-secret.sh does NOT apply to the optional NVIDIA key: that
# script only provisions capabilities/<name>/service.toml-backed secrets, and this is a tools/
# script, not a capability. The cloud key still goes through Vaultwarden (README.md#secrets).
#
# Backend selection (semantic extraction of docs, on top of the always-free AST pass):
# see tools/graphify.env.example. Local-first: defaults to oMLX, falls back to the
# original code-only behavior if the configured backend isn't actually reachable --
# this script never hard-fails just because a backend isn't running.
#
# Usage: tools/graphify.sh
# Check only: tools/graphify.sh --check
set -e

_lib="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/lib" && pwd)"
source "$_lib/paths.sh"

cd "$SJEL_ROOT"

_cfg="$SJEL_PERSONAL_ROOT/config/graphify.env"
if [ -f "$_cfg" ]; then
  source "$_cfg"
fi
# GRAPHIFY_BACKENDS is an ordered preference list; the first candidate that resolves a key AND
# answers a probe wins. GRAPHIFY_BACKEND stays the single-value spelling every older config uses,
# so nothing that set one value has to learn a new key.
#
# A chain rather than one value because the two useful cloud options differ in KIND, not quality:
# nvidia-nim is a free tier that can simply run out, and opencode-zen-go is paid-but-cheap. Trying
# free first and falling through when it is exhausted is the whole behaviour worth having, and a
# human choosing again each time it lapses is the thing this replaces.
GRAPHIFY_BACKENDS="${GRAPHIFY_BACKENDS:-${GRAPHIFY_BACKEND:-omlx}}"

# --- backend endpoints come from the inference registry, not from here ---------------------
#
# libs/inference already declares every backend this deployment has, by id, with base_url and the
# file holding its key: <overlay>/config/inference.json (schema in
# libs/inference/inference.config.example.json). Reading it means a new provider is one registry
# entry rather than another elif in this script, and it means graphify and libs/inference cannot
# disagree about where a key lives — which they would, silently, the first time one moved.
#
# Keys are read from the file the registry names and are NEVER echoed, interpolated into a URL, or
# passed as an argv element. tools/materialize-inference-key is the only thing that writes those
# files, straight from `bw get notes` at mode 0600 (README.md#secrets).
_inference_json="$SJEL_PERSONAL_ROOT/config/inference.json"

_registry_field() {  # _registry_field <backend-id> <field>
  [ -f "$_inference_json" ] || return 1
  jq -er --arg b "$1" --arg f "$2" '.backends[$b][$f] // empty' "$_inference_json" 2>/dev/null
}

# The key bytes for a backend, on stdout. Non-zero when the registry names no key file or the file
# is absent — "not provisioned" is a normal answer here, and it is what makes the chain fall
# through to the next candidate rather than fail the run.
_registry_key() {  # _registry_key <backend-id>
  local rel expanded
  rel="$(_registry_field "$1" api_key_file)" || return 1
  case "$rel" in
    "~"/*) expanded="$HOME/${rel#"~/"}" ;;
    /*)    expanded="$rel" ;;
    *)     expanded="$SJEL_PERSONAL_ROOT/config/$rel" ;;
  esac
  [ -f "$expanded" ] || return 1
  # A materialized key file holds the secret and nothing else; oMLX's settings.json is JSON and
  # keeps it under .auth.api_key. Both are registry-declared paths, so the shape is decided by
  # what the file is, not by which backend asked.
  case "$expanded" in
    *.json) jq -er '.auth.api_key | select(type == "string" and length > 0)' "$expanded" 2>/dev/null ;;
    *)      tr -d '\r\n' < "$expanded" ;;
  esac
}

# try_backend <id> — 0 when this candidate is usable, with the child environment set for it.
# Prints one line on stderr when it is not, because a chain that falls through silently is a chain
# nobody can debug: "which one am I actually on" must never need a code read.
#
# Everything except ollama speaks the OpenAI chat-completions wire format, so it goes through
# graphify's `openai` backend with a different base_url. `mlx` stays an alias for older configs.
try_backend() {
  local id="$1" reg base model key ctx fitted
  case "$id" in
    none) return 1 ;;
    ollama)
      base="$(_registry_field ollama base_url || echo 'http://localhost:11434')"
      if ! curl -sf --max-time 2 "$base" >/dev/null 2>&1; then
        echo "graphify.sh: ollama declared but no server at $base (start it with 'ollama serve')" >&2
        return 1
      fi
      _real_backend="ollama"
      return 0
      ;;
  esac
  # Registry ids and the spellings a config may use are not identical; nvidia has been the
  # short form in this file since before the registry existed.
  case "$id" in
    nvidia) reg="nvidia-nim" ;;
    mlx)    reg="omlx" ;;
    *)      reg="$id" ;;
  esac
  base="$(_registry_field "$reg" base_url)" || {
    echo "graphify.sh: '$id' is not declared in $_inference_json — skipping" >&2
    return 1
  }
  if ! key="$(_registry_key "$reg")" || [ -z "$key" ]; then
    # The expected case for a chain, not an error: a provisioned-later provider drops out until
    # its key exists. Named so the reason is obvious without reading the registry.
    echo "graphify.sh: '$id' has no key yet (tools/materialize-inference-key $reg) — skipping" >&2
    return 1
  fi
  model="${GRAPHIFY_MODEL:-$(_registry_field "$reg" model || true)}"
  # Header via stdin, never argv: a key in a command line is readable by every process on the box.
  if ! printf 'Authorization: Bearer %s\n' "$key" |
       curl -sf --max-time 5 -H @- "${base%/}/models" >/dev/null 2>&1; then
    echo "graphify.sh: '$id' did not answer at ${base%/}/models — skipping" >&2
    return 1
  fi
  _real_backend="openai"
  OPENAI_BASE_URL="$base"
  OPENAI_API_KEY="$key"
  [ -n "$model" ] && { OPENAI_MODEL="$model"; GRAPHIFY_MODEL="$model"; }
  export OPENAI_BASE_URL OPENAI_API_KEY
  [ -n "$model" ] && export OPENAI_MODEL
  # Local servers declare a context window small enough that graphify's default chunk does not
  # fit; hosted ones on this chain carry 200k-1M windows and need no fitting at all.
  if [ "$reg" = omlx ]; then
    _omlx_settings="$(_registry_field omlx api_key_file || echo "$HOME/.omlx/settings.json")"
    case "$_omlx_settings" in "~"/*) _omlx_settings="$HOME/${_omlx_settings#"~/"}" ;; esac
    # Fit the chunk to the server's context window, derived rather than guessed.
    #
    # graphify chunks at 60,000 INPUT tokens by default, which is a cloud-sized number. A local
    # server declaring a 32,768-token window rejects every such chunk with `exceeded context
    # (BadRequestError)` before the model reads a token. One window has to hold the system
    # prompt, the chunk AND the response, so the budget is what remains after reserving for the
    # other two; output is pinned here as well (graphify honours GRAPHIFY_MAX_OUTPUT_TOKENS) so
    # the arithmetic is over three known numbers rather than two known and one hoped-for.
    #
    # NECESSARY, NOT SUFFICIENT — say this out loud, because a run that gets further looks like
    # a run that works. Fitting the window removed the depth-0 wall and roughly tripled the chunk
    # count, and a full-corpus run on 2026-08-06 still failed: 31 hollow responses, 8 rejections
    # that reappeared at deeper bisections, 4 empty-choices aborts. On that machine the binding
    # limit is the memory guard, not the window, and no chunk size fixes that. This code makes a
    # capable local backend work; it does not make an over-sized one fit. See the graphify.env
    # in the active overlay for how that decision was recorded.
    _omlx_ctx="$(jq -er '.sampling.max_context_window | numbers' "$_omlx_settings" 2>/dev/null || echo 32768)"
    GRAPHIFY_MAX_OUTPUT_TOKENS="${GRAPHIFY_MAX_OUTPUT_TOKENS:-8192}"
    _prompt_reserve=4096
    _fitted=$(( _omlx_ctx - GRAPHIFY_MAX_OUTPUT_TOKENS - _prompt_reserve ))
    [ "$_fitted" -lt 2048 ] && _fitted=2048   # a window too small to chunk into is a config problem, not a reason to send 0
    GRAPHIFY_TOKEN_BUDGET="${GRAPHIFY_TOKEN_BUDGET:-$_fitted}"
    export GRAPHIFY_MAX_OUTPUT_TOKENS
  fi
  return 0
}

# Walk the chain. First candidate that resolves wins; `none` anywhere in it is an explicit
# code-only decision and stops the walk rather than being skipped over, because "I decided against
# a semantic pass" and "nothing answered" are different states and must not report the same.
_use_backend=0
_real_backend=""
GRAPHIFY_BACKEND=""
for _candidate in $GRAPHIFY_BACKENDS; do
  if [ "$_candidate" = none ]; then
    GRAPHIFY_BACKEND=none
    break
  fi
  if try_backend "$_candidate"; then
    GRAPHIFY_BACKEND="$_candidate"
    _use_backend=1
    break
  fi
done
if [ "$_use_backend" = 0 ] && [ "$GRAPHIFY_BACKEND" != none ]; then
  echo "graphify.sh: no backend in '$GRAPHIFY_BACKENDS' resolved -- falling back to code-only." >&2
  GRAPHIFY_BACKEND="${GRAPHIFY_BACKEND:-none}"
fi

if [ "${1:-}" = "--check" ]; then
  if [ "$_use_backend" = "1" ]; then
    # The chain, not just the winner: "nvidia is down so this ran on the paid fallback" is the one
    # thing an operator needs from this command, and reporting only the winner hides it.
    echo "graphify.sh: chain='$GRAPHIFY_BACKENDS' -> backend=$GRAPHIFY_BACKEND wire=$_real_backend model=${GRAPHIFY_MODEL:-${OPENAI_MODEL:-<provider-default>}} reachable"
    # The chunk budget is the setting that silently emptied the semantic pass, so --check says
    # it out loud rather than leaving it to a full run to discover.
    [ -n "${GRAPHIFY_TOKEN_BUDGET:-}" ] \
      && echo "graphify.sh: token-budget=$GRAPHIFY_TOKEN_BUDGET out=${GRAPHIFY_MAX_OUTPUT_TOKENS:-<default>} (fitted to a ${_omlx_ctx:-?}-token context window)"
    exit 0
  fi
  echo "graphify.sh: semantic backend unavailable; normal runs would use code-only mode" >&2
  exit 1
fi

# Pinned via `uv tool run`, deliberately NOT the bare `graphify` on PATH -- see
# upstreams.toml [graphify] for why that distinction matters here.
echo "--- graphify: rebuilding code-dependency graph ---"
if [ "$_use_backend" = "1" ]; then
  echo "--- graphify: semantic extraction via backend '$GRAPHIFY_BACKEND' ---"
  _model_args=()
  [ -n "${GRAPHIFY_MODEL:-}" ] && _model_args=(--model "$GRAPHIFY_MODEL")
  # graphify defaults to four semantic chunks in flight, and its own --help says to set 1
  # for local LLMs. A local server answers one request at a time anyway, so four in flight
  # buys no throughput and does risk the memory guard on a machine also running the model.
  # Upstream's documented setting for this deployment shape, correct on its own terms.
  #
  # --token-budget is the one that was actually emptying the graph: graphify chunks at 60,000
  # input tokens by default, which does not fit a 32,768-token local context window, so every
  # chunk came back BadRequestError before the model read any of it. Passed for every local
  # backend, from the value derived against the server's declared window above.
  _tuning_args=()
  case "$GRAPHIFY_BACKEND" in
    omlx|mlx|ollama) _tuning_args=(--max-concurrency 1) ;;
  esac
  [ -n "${GRAPHIFY_TOKEN_BUDGET:-}" ] && _tuning_args+=(--token-budget "$GRAPHIFY_TOKEN_BUDGET")
  uv tool run --from "graphifyy[openai]" graphify extract . --backend "$_real_backend" "${_model_args[@]}" "${_tuning_args[@]}"
else
  # `update`, not `extract --code-only`: the latter has a confirmed-broken --code-only
  # flag on every graphifyy release through 0.9.10 (silently demands an LLM key for doc
  # files anyway); `update` is the older, established command and has always been
  # code-only by design.
  uv tool run --from graphifyy graphify update .
fi
# What actually came out, read off the artifact rather than scraped from graphify's log.
# graphify preserves a usable AST graph and exits 0 even when semantic extraction fails, so
# exit status alone cannot tell the two apart.
#
# `_origin` is the signal: graphify stamps every node and edge with the pass that produced
# it, and an AST-only graph carries `ast` on all of them. Counting anything-but-ast is
# deliberately not a match against a specific label -- upstream may name the semantic pass
# whatever it likes, and asking "did anything other than the AST pass contribute" survives
# that. Two fields that look like they would work and do not: `summary` does not exist in
# this schema at all, so any check against it reports code-only forever; `community_name`
# is populated on a pure AST run too (it is slugified from paths and symbols).
report_outcome() {
  local graph="$SJEL_ROOT/graphify-out/graph.json" total=0 semantic=0 hyper=0

  if [ ! -f "$graph" ]; then
    echo "--- graphify: unavailable -- no graph.json was produced ---" >&2
    return 1
  fi
  total="$(jq -r '.nodes | length' "$graph" 2>/dev/null || echo 0)"
  semantic="$(jq -r '[.nodes[], .links[]? | select(._origin != "ast")] | length' "$graph" 2>/dev/null || echo 0)"
  hyper="$(jq -r '.hyperedges | length' "$graph" 2>/dev/null || echo 0)"

  if [ "$_use_backend" != "1" ]; then
    echo "--- graphify: code-only -- $total nodes, no semantic pass requested ---"
  elif [ "$semantic" -eq 0 ] && [ "$hyper" -eq 0 ]; then
    echo "--- graphify: code-only -- $total nodes, semantic pass via '$GRAPHIFY_BACKEND' contributed nothing ---" >&2
  else
    echo "--- graphify: semantic -- $total nodes, $semantic non-AST element(s), $hyper hyperedge(s) via '$GRAPHIFY_BACKEND' ---"
  fi
  echo "    see graphify-out/graph.html, GRAPH_REPORT.md"
}

report_outcome
