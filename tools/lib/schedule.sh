#!/bin/bash
# tools/lib/schedule.sh — the one parser for a service.toml `schedule` field.
#
# Extracted rather than written twice, for the same reason tools/lib/toml.sh is the only TOML
# parser: two readers of one field drift, and the drift here would be silent — a manifest the
# schema gate accepts and install-persistence then refuses, or worse, the reverse.
#
# Two callers: tools/check-service-tomls.sh (is this manifest declarable at all) and
# tools/service-runner.sh (what interval does it actually render to).

# schedule_seconds — the manifest's `schedule` as a whole number of seconds, on stdout. Non-zero
# with the reason (also on stdout, so a caller can report it) when the spec is not one we accept.
#
# A duration, not a cron expression. Both backends want a duration anyway — launchd's
# StartInterval is seconds, systemd's OnUnitActiveSec is a time span — so cron syntax would mean
# teaching a manifest a second scheduling language and then translating out of it. "Every N" is
# what the consumers actually have (a feed sweep, a supply-chain gate). Wall-clock alignment
# ("03:00 daily") is a different feature with a different backend mapping, and belongs in a later
# field rather than a dialect half-supported here.
schedule_seconds() {  # <spec>
  local spec="$1" n unit
  n="${spec%[mhd]}"; unit="${spec#"$n"}"
  case "$unit" in
    m|h|d) ;;
    *) echo "schedule = \"$spec\" — expected <N>m, <N>h or <N>d (minutes, hours, days)"; return 1 ;;
  esac
  case "$n" in
    ''|*[!0-9]*) echo "schedule = \"$spec\" — '$n' is not a whole number"; return 1 ;;
  esac
  # Base 10 explicitly: bash arithmetic reads a leading zero as octal, so "0010h" was 8 hours and
  # "0008h" an arithmetic error. Found 2026-10-02 by tools/lib/schedule-cases.tsv, the table this
  # parser and tools/sjel-cli/src/schedule.rs now share.
  n=$((10#$n))
  [ "$n" -gt 0 ] || { echo "schedule = \"$spec\" — must be greater than zero"; return 1; }
  case "$unit" in
    m) echo "$((n * 60))" ;;
    h) echo "$((n * 3600))" ;;
    d) echo "$((n * 86400))" ;;
  esac
}
