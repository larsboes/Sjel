<script lang="ts">
  import { projects, PROJECT_STATUSES, TASK_STATUSES, type ProjectTask } from "$lib/api";
  import StateLine from "$lib/StateLine.svelte";
  import { link } from "$lib/nav";
  import { linkTargets } from "./database";

  /**
   * Records as a database, the way an Obsidian Base shows them: one row per record, one column
   * per frontmatter key, each cell edited in place. Every key can be a column (dynamic columns,
   * chosen 2026-10-18); the keys the server checks get a matching control. A cell writes one
   * property and the row is replaced with what the server rendered.
   */
  let {
    kinds,
    filter,
    columns: initial,
    storeKey,
    stamp = 0,
    onchange,
  }: {
    kinds: ProjectTask["kind"][];
    /** Which rows belong to this view; all of `kinds` when absent. */
    filter?: (r: ProjectTask) => boolean;
    /** The columns shown before the operator picks any. */
    columns: string[];
    /** Where the view's columns, grouping and sort are kept in this browser. */
    storeKey: string;
    /** Bump to reload, after a write made elsewhere on the page. */
    stamp?: number;
    onchange?: () => void;
  } = $props();

  type Sort = { key: string; dir: 1 | -1 };
  // svelte-ignore state_referenced_locally
  const STORE = `sjel.projects.${storeKey}`;
  /** Shown, never edited: the server derives these from the path or refuses them. */
  const READ_ONLY = new Set(["project", "open", "type", "moved_to"]);
  const DATES = new Set(["due", "scheduled", "start", "completed"]);
  const statuses = (r: ProjectTask): readonly string[] =>
    r.kind === "task" ? TASK_STATUSES : r.kind === "project" ? PROJECT_STATUSES : [];

  let rows = $state<ProjectTask[]>([]);
  /** Open tasks per project note, for the `open` column of a projects view. */
  let openCount = $state<Map<string, number>>(new Map());
  let loadState = $state<"loading" | "error" | "ready">("loading");
  let error = $state<string | null>(null);
  let saving = $state<string | null>(null);
  let query = $state("");
  const saved = load();
  // svelte-ignore state_referenced_locally
  let columns = $state<string[]>(saved.columns ?? initial);
  let groupBy = $state<string>(saved.groupBy ?? "");
  let sort = $state<Sort | null>(saved.sort ?? null);
  let showDone = $state<boolean>(saved.showDone ?? false);

  function load(): { columns?: string[]; groupBy?: string; sort?: Sort | null; showDone?: boolean } {
    try {
      return JSON.parse(localStorage.getItem(STORE) ?? "{}");
    } catch {
      return {};
    }
  }
  $effect(() => {
    localStorage.setItem(STORE, JSON.stringify({ columns, groupBy, sort, showDone }));
  });

  const isDone = (r: ProjectTask) => r.done || ["done", "cancelled"].includes(r.fields.status ?? "");
  const visible = $derived(rows.filter((r) => !filter || filter(r)));

  /** Every key a row in this view carries, the initial columns first. */
  const keys = $derived.by(() => {
    const seen = new Set(["project", ...(kinds.includes("project") ? ["open"] : []), ...initial]);
    for (const r of visible) for (const k of Object.keys(r.fields)) if (k !== "done") seen.add(k);
    return [...seen];
  });
  const shown = $derived(columns.filter((c) => keys.includes(c)));

  const cell = (r: ProjectTask, key: string) =>
    key === "project" ? r.project : key === "open" ? String(openCount.get(r.title) ?? 0) : (r.fields[key] ?? "");
  /** Done last, dated before undated, then priority. ISO dates compare as plain text;
   *  `localeCompare` would sort a sentinel like "~" before digits. */
  const byDue = (a?: string, b?: string) => (a === b ? 0 : !a ? 1 : !b ? -1 : a < b ? -1 : 1);
  const natural = (a: ProjectTask, b: ProjectTask) =>
    Number(isDone(a)) - Number(isDone(b)) ||
    byDue(a.fields.due, b.fields.due) ||
    a.priority - b.priority ||
    a.title.localeCompare(b.title);

  const ordered = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const picked = visible.filter(
      (r) =>
        (showDone || !isDone(r)) &&
        (!q || r.title.toLowerCase().includes(q) || Object.values(r.fields).some((v) => v.toLowerCase().includes(q))),
    );
    const s = sort;
    return picked.toSorted(
      s
        ? (a, b) => {
            const x = s.key === "title" ? a.title : cell(a, s.key);
            const y = s.key === "title" ? b.title : cell(b, s.key);
            // Blank last in either direction: an unset due date is not the earliest.
            return x === y ? natural(a, b) : !x ? 1 : !y ? -1 : s.dir * x.localeCompare(y, undefined, { numeric: true });
          }
        : natural,
    );
  });
  const hiddenDone = $derived(visible.filter(isDone).length);

  const groups = $derived.by(() => {
    if (!groupBy) return [{ name: "", rows: ordered }];
    const map = new Map<string, ProjectTask[]>();
    for (const r of ordered) {
      const name = cell(r, groupBy) || "—";
      map.set(name, [...(map.get(name) ?? []), r]);
    }
    const known: readonly string[] = groupBy === "status" ? [...TASK_STATUSES, ...PROJECT_STATUSES] : [];
    const rank = (n: string) => (known.includes(n) ? known.indexOf(n) : known.length);
    return [...map]
      .map(([name, rows]) => ({ name, rows }))
      .toSorted((a, b) => rank(a.name) - rank(b.name) || a.name.localeCompare(b.name));
  });

  async function reload() {
    try {
      const wanted: ProjectTask["kind"][] = kinds.includes("project") && !kinds.includes("task") ? [...kinds, "task"] : kinds;
      const loaded = (await Promise.all(wanted.map((k) => projects.records(k)))).flat();
      const counts = new Map<string, number>();
      for (const t of loaded)
        if (t.kind === "task" && !t.done) for (const p of linkTargets(t.fields.projects)) counts.set(p, (counts.get(p) ?? 0) + 1);
      openCount = counts;
      rows = loaded.filter((r) => kinds.includes(r.kind));
      loadState = "ready";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      loadState = "error";
    }
  }

  $effect(() => {
    void stamp;
    void reload();
  });

  /** Write one property. The row changes at once and rolls back if the server refuses. */
  async function set(r: ProjectTask, key: string, value: string) {
    if ((r.fields[key] ?? "") === value) return;
    const at = () => rows.findIndex((x) => x.id === r.id);
    rows[at()] = { ...r, fields: { ...r.fields, [key]: value } };
    saving = r.id;
    error = null;
    try {
      rows[at()] = await projects.setProperty(r.id, key, value);
      onchange?.();
    } catch (e) {
      rows[at()] = r;
      error = `${r.title}: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      saving = null;
    }
  }

  function toggleSort(key: string) {
    sort = sort?.key !== key ? { key, dir: 1 } : sort.dir === 1 ? { key, dir: -1 } : null;
  }

  function toggleColumn(key: string) {
    columns = columns.includes(key) ? columns.filter((c) => c !== key) : [...columns, key];
  }

  const arrow = (key: string) => (sort?.key === key ? (sort.dir === 1 ? " ↑" : " ↓") : "");
</script>

<div class="toolbar">
  <input type="search" placeholder="Filter" aria-label="Filter rows" bind:value={query} />
  {#if hiddenDone > 0}
    <label class="inline"><input type="checkbox" bind:checked={showDone} /> Done · {hiddenDone}</label>
  {/if}
  <label class="inline">
    Group
    <select bind:value={groupBy} aria-label="Group by">
      <option value="">none</option>
      {#each keys as k (k)}<option value={k}>{k}</option>{/each}
    </select>
  </label>
  <details class="columns">
    <summary>Columns</summary>
    <div class="menu">
      {#each keys as k (k)}
        <label><input type="checkbox" checked={columns.includes(k)} onchange={() => toggleColumn(k)} /> {k}</label>
      {/each}
    </div>
  </details>
  <span class="count">{ordered.length} row{ordered.length === 1 ? "" : "s"}</span>
</div>

<StateLine state={loadState} message={loadState === "error" ? (error ?? undefined) : "Reading…"} onRetry={reload} />
{#if error && loadState === "ready"}<p class="error" role="alert">{error}</p>{/if}

{#if loadState === "ready"}
  <div class="scroll">
    <table>
      <thead>
        <tr>
          {#if kinds.includes("task")}<th class="check" scope="col"><span class="sr-only">Done</span></th>{/if}
          <th scope="col" aria-sort={sort?.key === "title" ? (sort.dir === 1 ? "ascending" : "descending") : undefined}>
            <button type="button" onclick={() => toggleSort("title")}>Name{arrow("title")}</button>
          </th>
          {#each shown as k (k)}
            <th scope="col" aria-sort={sort?.key === k ? (sort.dir === 1 ? "ascending" : "descending") : undefined}>
              <button type="button" onclick={() => toggleSort(k)}>{k}{arrow(k)}</button>
            </th>
          {/each}
        </tr>
      </thead>
      {#each groups as g (g.name)}
        <tbody>
          {#if groupBy}
            <tr class="group">
              <th colspan={shown.length + 2} scope="rowgroup">{g.name} <span class="count">{g.rows.length}</span></th>
            </tr>
          {/if}
          {#each g.rows as r (r.id)}
            <tr class:done={isDone(r)} class:saving={saving === r.id}>
              {#if kinds.includes("task")}
                <td class="check">
                  {#if r.kind === "task"}
                    <input
                      type="checkbox"
                      aria-label={`Done: ${r.title}`}
                      checked={r.done}
                      onchange={() => set(r, "status", r.done ? "open" : "done")}
                    />
                  {/if}
                </td>
              {/if}
              <!-- A task or note opens its own body; a link to its project alone went nowhere from inside that project. -->
              <td class="title"><a href={link(`/projects?project=${encodeURIComponent(r.project)}${r.kind === "project" ? "" : `&open=${encodeURIComponent(r.id)}`}`)}>{r.title}</a></td>
              {#each shown as k (k)}
                {@const label = `${k}: ${r.title}`}
                <td>
                  {#if READ_ONLY.has(k)}
                    <span class="ro">{cell(r, k)}</span>
                  {:else if k === "status" && statuses(r).length > 0}
                    <select aria-label={label} value={r.fields.status ?? ""} onchange={(e) => set(r, k, e.currentTarget.value)}>
                      {#if !statuses(r).includes(r.fields.status ?? "")}<option value={r.fields.status ?? ""}>{r.fields.status || "—"}</option>{/if}
                      {#each statuses(r) as s (s)}<option value={s}>{s}</option>{/each}
                    </select>
                  {:else if k === "priority"}
                    <select aria-label={label} value={r.fields.priority ?? ""} onchange={(e) => set(r, k, e.currentTarget.value)}>
                      {#if !r.fields.priority}<option value="">—</option>{/if}
                      {#each ["1", "2", "3"] as p (p)}<option value={p}>P{p}</option>{/each}
                    </select>
                  {:else if DATES.has(k)}
                    <input type="date" aria-label={label} value={r.fields[k] ?? ""} onchange={(e) => set(r, k, e.currentTarget.value)} />
                  {:else}
                    <input
                      aria-label={label}
                      value={r.fields[k] ?? ""}
                      onchange={(e) => set(r, k, e.currentTarget.value.trim())}
                      onkeydown={(e) => e.key === "Enter" && e.currentTarget.blur()}
                    />
                  {/if}
                </td>
              {/each}
            </tr>
          {/each}
        </tbody>
      {/each}
    </table>
    {#if ordered.length === 0}<p class="hint">{query ? "Nothing matches." : "Nothing open."}</p>{/if}
  </div>
{/if}

<style>
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.6rem;
    margin-bottom: 0.6rem;
  }

  .toolbar input[type="search"] {
    flex: 1 1 12rem;
  }

  .inline {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }

  .columns {
    position: relative;
    font-size: var(--text-sm);
  }

  .columns summary {
    cursor: pointer;
    color: var(--text-secondary);
  }

  .menu {
    position: absolute;
    right: 0;
    z-index: 2;
    display: grid;
    gap: 0.2rem;
    min-width: 10rem;
    max-height: 20rem;
    overflow-y: auto;
    padding: 0.5rem;
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
  }

  .count,
  .hint,
  .ro {
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 400;
  }

  .scroll {
    overflow-x: auto;
  }

  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--text-sm);
  }

  th,
  td {
    padding: 0.15rem 0.4rem;
    border-bottom: 1px solid var(--rule);
    text-align: left;
    white-space: nowrap;
  }

  thead th button {
    all: unset;
    cursor: pointer;
    color: var(--text-secondary);
    font-weight: 600;
  }

  .group th {
    padding-top: 0.9rem;
    font-weight: 600;
  }

  td.title {
    white-space: normal;
    min-width: 14rem;
  }

  td.title a {
    color: inherit;
    text-decoration: none;
  }

  td.title a:hover {
    text-decoration: underline;
  }

  .check {
    width: 1.5rem;
  }

  /* A cell looks like text until it is touched, as in a Notion table. */
  td input:not([type="checkbox"]),
  td select {
    width: 100%;
    min-width: 6rem;
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    color: inherit;
    font: inherit;
    padding: 0.2rem 0.3rem;
  }

  td input:not([type="checkbox"]):hover,
  td select:hover,
  td input:focus,
  td select:focus {
    border-color: var(--input-border);
    background: var(--input-bg);
  }

  tr.done td.title {
    color: var(--text-secondary);
    text-decoration: line-through;
  }

  tr.saving {
    opacity: 0.6;
  }

  .error {
    color: var(--danger);
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
  }
</style>
