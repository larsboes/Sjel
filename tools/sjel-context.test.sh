#!/bin/bash
set -euo pipefail

tools_dir="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
repo="$(mktemp -d)"
trap 'rm -rf "$repo"' EXIT

git -C "$repo" init -q -b main
mkdir "$repo/tools" "$repo/docs"
mkdir -p "$repo/Packs/example"
cp "$tools_dir/sjel-context" "$repo/tools/sjel-context"
touch "$repo/docs/guide.md" "$repo/other.md" "$repo/Packs/example/README.md"
git -C "$repo" add -- .
git -C "$repo" -c core.hooksPath=/dev/null -c user.name=Fixture \
  -c user.email=fixture@example.invalid commit -qm fixture

printf 'changed\n' | tee "$repo/docs/guide.md" > "$repo/other.md"
for number in {01..12}; do
  touch "$repo/docs/new-$number.md"
done

directory_context="$("$repo/tools/sjel-context" on docs)"
grep -qx 'branch=main' <<< "$directory_context"
grep -qx 'changes=tracked:2 untracked:12' <<< "$directory_context"
grep -qx 'changes_omitted=1' <<< "$directory_context"
[[ "$(grep -c '^change=' <<< "$directory_context")" -eq 12 ]]
[[ "$directory_context" != *other.md* ]]

file_context="$("$repo/tools/sjel-context" on docs/guide.md)"
grep -qx 'change= M docs/guide.md' <<< "$file_context"
[[ "$(grep -c '^change=' <<< "$file_context")" -eq 1 ]]
[[ "$file_context" != *changes_omitted=* ]]

printf 'changed\n' > "$repo/Packs/example/README.md"
pack_context="$("$repo/tools/sjel-context" on example)"
grep -qx 'change= M Packs/example/README.md' <<< "$pack_context"
[[ "$(grep -c '^change=' <<< "$pack_context")" -eq 1 ]]

printf 'sjel-context target status passed\n'
