#!/bin/bash
# Planted-tree regression tests for check-obsidian-plugins.sh.
#
# The gate walks `find . -name manifest.json` from its working directory, so each case is a
# throwaway tree the test cds into. The red path is proven here rather than assumed: an
# instrument that answers the same for a known-bad input as for a known-good one is
# measuring something other than what was asked.
#
# Each planted bad input below is a real mistake with a real consequence, not a synthetic
# string: the browser transport is a registry read the renderer withholds, the write is a
# second writer of keys vault only computes, the node require is a plugin that loads on a
# desktop and not on a phone, and the port literal is the thing that goes stale.
set -uo pipefail

CHECK="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)/check-obsidian-plugins.sh"

SCRATCH="$(mktemp -d)"
trap 'rm -rf "$SCRATCH"' EXIT

fails=0

tree() { # tree <case-name> -> echoes the fresh plugin directory
  local root="$SCRATCH/$1"
  rm -rf "$root"
  mkdir -p "$root/plugin"
  printf '%s' "$root"
}

manifest() { # manifest <root> [desktop-only]
  cat > "$1/plugin/manifest.json" <<JSON
{ "id": "planted", "name": "Planted", "version": "1.0.0", "minAppVersion": "1.5.0",
  "description": "planted", "author": "test", "isDesktopOnly": ${2:-false} }
JSON
}

run() { # run <tree-root> -> gate output on stdout+stderr, exit status in $status
  out=$(cd "$1" && "$CHECK" 2>&1)
  status=$?
}

expect_pass() { # expect_pass <description> <tree-root>
  run "$2"
  if [ "$status" -ne 0 ]; then
    echo "FAIL: $1 should pass, got exit $status:"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

expect_fail_with() { # expect_fail_with <description> <tree-root> <substring>
  run "$2"
  if [ "$status" -eq 0 ] || ! printf '%s' "$out" | grep -qF "$3"; then
    echo "FAIL: $1 should fail naming '$3', got exit $status:"
    printf '%s\n' "$out" | sed 's/^/    /'
    fails=$((fails + 1))
  fi
}

# --- the green path, which every red case below is one edit away from ------

root=$(tree clean)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin, requestUrl } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  async onload() {
    const rows = await requestUrl({ url: DEFAULTS.registryUrl, method: "GET", throw: false });
    this.rows = rows.json;
  }
};
JS
expect_pass "a read-only plugin on requestUrl with one bootstrap address" "$root"

# --- R1: the browser transport ---------------------------------------------
#
# This is the mistake the gate exists for. It is one word away from the green
# case above and looks more idiomatic, and the renderer withholds the reply of
# every capability without a CORS layer, sjel-status' registry first.

root=$(tree browser-fetch)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  async onload() {
    const response = await fetch(DEFAULTS.registryUrl);
    this.rows = await response.json();
  }
};
JS
expect_fail_with "a browser fetch is refused" "$root" "browser-context HTTP"

root=$(tree browser-xhr)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  onload() {
    const xhr = new XMLHttpRequest();
    xhr.open("GET", DEFAULTS.registryUrl);
    xhr.send();
  }
};
JS
expect_fail_with "an XHR is the same mistake spelled differently" "$root" "browser-context HTTP"

# A member call named fetch is somebody else's API, not the global.
root=$(tree member-fetch-allowed)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  onload() {
    this.cache.fetch(DEFAULTS.registryUrl);
  }
};
JS
expect_pass "a method named fetch on an object is not the global" "$root"

# The same global reached through the object that carries it. `window.fetch` is what an
# editor completes to, and it is the identical request with the identical Origin.
root=$(tree window-fetch)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  async onload() {
    const response = await window.fetch(DEFAULTS.registryUrl);
    this.rows = await response.json();
  }
};
JS
expect_fail_with "fetch through window is the same request" "$root" "browser-context HTTP"

# --- R2: a second writer of the vault -------------------------------------

root=$(tree writes-frontmatter)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin } = require("obsidian");
module.exports = class extends Plugin {
  async onload() {
    const file = this.app.workspace.getActiveFile();
    await this.app.fileManager.processFrontMatter(file, (matter) => {
      matter.last_contact = "2026-09-09";
    });
  }
};
JS
expect_fail_with "writing frontmatter is refused" "$root" "read-only"

# Rendering is not writing. `createEl`, `createDiv` and `append` are what every plugin does
# to build its pane, and a gate that reports them is a gate somebody deletes.
root=$(tree dom-calls-allowed)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  onload() {
    const row = this.containerEl.createEl("tr");
    row.createEl("td", { text: DEFAULTS.registryUrl });
    this.containerEl.append(row);
  }
};
JS
expect_pass "building a pane is not writing to the vault" "$root"

root=$(tree writes-over-http)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin, requestUrl } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  async onload() {
    await requestUrl({ url: DEFAULTS.registryUrl, method: "POST", body: "{}" });
  }
};
JS
expect_fail_with "a POST is a write however it is spelled" "$root" "read-only"

# However it is spelled includes the quote around it. Nothing in this repository formats a
# plugin, so which quote a file uses is the author's habit and not a rule.
root=$(tree writes-over-http-single-quoted)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin, requestUrl } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  async onload() {
    await requestUrl({ url: DEFAULTS.registryUrl, method: 'PUT', body: '{}' });
  }
};
JS
expect_fail_with "a single-quoted verb is the same write" "$root" "read-only"

# --- R3: the manifest and the source disagreeing about mobile -------------

root=$(tree node-require-on-mobile)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin } = require("obsidian");
const fs = require("node:fs");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  onload() {
    this.overlay = fs.readFileSync("/etc/hosts", "utf8");
    this.url = DEFAULTS.registryUrl;
  }
};
JS
expect_fail_with "a node require under isDesktopOnly: false is refused" "$root" "isDesktopOnly is false"

# The same source with the manifest telling the truth is allowed: the rule is that the two
# agree, not that node is forbidden.
root=$(tree node-require-declared)
manifest "$root" true
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin } = require("obsidian");
const fs = require("node:fs");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  onload() {
    this.overlay = fs.readFileSync("/etc/hosts", "utf8");
    this.url = DEFAULTS.registryUrl;
  }
};
JS
expect_pass "a desktop-only plugin may require node" "$root"

# --- R4: the address that goes stale --------------------------------------

root=$(tree second-address)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin, requestUrl } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  async onload() {
    this.registry = DEFAULTS.registryUrl;
    this.people = await requestUrl({ url: "http://127.0.0.1:8094/api/people", method: "GET" });
  }
};
JS
expect_fail_with "a second hardcoded address is refused" "$root" "one bootstrap setting"

# The exemption is the setting line, not the word. A trailing comment naming registryUrl
# does not turn a second address into the bootstrap one.
root=$(tree second-address-mentioning-the-setting)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin, requestUrl } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin {
  async onload() {
    this.registry = DEFAULTS.registryUrl;
    this.plans = await requestUrl({ url: "http://127.0.0.1:8086/api/plans" }); // beside registryUrl
  }
};
JS
expect_fail_with "naming the setting in a comment is not the setting" "$root" "one bootstrap setting"

# --- the gate must not report green over an empty sweep -------------------

root=$(tree no-plugin-at-all)
mkdir -p "$root/docs"
echo "# no plugin here" > "$root/docs/readme.md"
expect_fail_with "an empty sweep is a broken gate, not a clean tree" "$root" \
  "no Obsidian plugin found"

# A manifest.json that is not an Obsidian manifest must not be counted as one, so a tree
# holding only web manifests still trips the empty-sweep guard.
root=$(tree web-manifest-only)
mkdir -p "$root/site"
echo '{ "name": "site", "start_url": "/" }' > "$root/site/manifest.json"
echo 'console.log("not a plugin");' > "$root/site/main.js"
expect_fail_with "a web app manifest is not an Obsidian plugin" "$root" \
  "no Obsidian plugin found"

# --- a nested checkout is not part of this tree ---------------------------

root=$(tree nested-checkout-pruned)
manifest "$root"
cat > "$root/plugin/main.js" <<'JS'
"use strict";
const { Plugin } = require("obsidian");
const DEFAULTS = { registryUrl: "http://127.0.0.1:8082/api/sjel-status/capabilities" };
module.exports = class extends Plugin { onload() { this.url = DEFAULTS.registryUrl; } };
JS
mkdir -p "$root/.claude/worktrees/wf-1/plugin"
cp "$root/plugin/manifest.json" "$root/.claude/worktrees/wf-1/plugin/manifest.json"
cat > "$root/.claude/worktrees/wf-1/plugin/main.js" <<'JS'
const response = await fetch("http://127.0.0.1:8094/api/people");
JS
expect_pass "a checkout nested under .claude/ is somebody else's tree" "$root"

# ---------------------------------------------------------------------------

if [ "$fails" -ne 0 ]; then
  echo "check-obsidian-plugins.test.sh: $fails case(s) FAILED" >&2
  exit 1
fi
echo "check-obsidian-plugins.test.sh: all cases passed"
