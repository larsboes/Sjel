#!/bin/bash
# Regression tests for tools/check-staged-rustfmt.sh: it judges the staged blob, takes the
# edition from the nearest Cargo.toml, and ignores files that are not staged Rust.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-staged-rustfmt.sh"
command -v rustfmt >/dev/null 2>&1 || { echo "skip: rustfmt is not installed"; exit 0; }

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT
cd "$SCRATCH" || exit 1
git init -q .
git config user.email test@example.invalid
git config user.name test
mkdir -p crate/src
printf '[package]\nname = "x"\nedition = "2021"\n' > crate/Cargo.toml

fails=0
status=0
run() { "$CHECK" 2>"$SCRATCH/stderr"; status=$?; }
fail() { echo "FAIL: $1"; fails=$((fails + 1)); }

printf 'fn main() {}\n' > crate/src/main.rs
git add crate
run
[ "$status" -eq 0 ] || fail "a formatted staged file should pass, got $status"

printf 'fn   main( ) {}\n' > crate/src/main.rs
git add crate/src/main.rs
run
[ "$status" -eq 1 ] || fail "an unformatted staged file should fail, got $status"
grep -qF "crate/src/main.rs is not formatted" "$SCRATCH/stderr" || fail "the refusal names the file"

printf 'fn main() {}\n' > crate/src/main.rs
git add crate/src/main.rs
printf 'fn   main( ) {}\n' > crate/src/main.rs
run
[ "$status" -eq 0 ] || fail "an unstaged edit must not block the staged blob, got $status"

# `async fn` is invalid in edition 2015, so a wrong edition would fail this file.
printf 'async fn run() {}\n' > crate/src/lib.rs
git add crate/src/lib.rs
run
[ "$status" -eq 0 ] || fail "the manifest edition applies, got $status"

git rm -q -f --cached crate/src/lib.rs crate/src/main.rs
printf 'not   rust\n' > notes.txt
git add notes.txt
run
[ "$status" -eq 0 ] || fail "non-Rust files are not checked, got $status"

mkdir -p research
printf 'fn   main( ) {}\n' > research/probe.rs
git add research/probe.rs
run
[ "$status" -eq 0 ] || fail "research/ is outside the CI format check, got $status"

[ "$fails" -eq 0 ] && echo "check-staged-rustfmt: all cases pass"
exit "$fails"
