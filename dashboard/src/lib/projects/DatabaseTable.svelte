<script lang="ts">
  import { projects, PROJECT_STATUSES, TASK_STATUSES, type ProjectTask } from "$lib/api";
  import StateLine from "$lib/StateLine.svelte";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import { linkTargets } from "./database";

  /**
   * Records as a database, the way an Obsidian Base shows them: table, board, list and calendar
   * views over `capabilities/projects` records, plus saved named views that keep a filter, sort,
   * group, layout and visible properties.
   */
  let {
    kinds,
    filter,
    columns: initial,
    storeKey,
    stamp = 0,
    onchange,
    onselect,
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
    onselect?: (r: ProjectTask) => void;
  } = $props();

  type Sort = { key: string; dir: 1 | -1 };
  type LayoutMode = "table" | "board" | "list" | "calendar";
  type SavedView = {
    name: string;
    layout: LayoutMode;
    columns: string[];
    groupBy: string;
    sort: Sort | null;
    showDone: boolean;
    query: string;
  };
  const CLASS_LABELS: Record<string, string> = {
    c0: "Public",
    c1: "Mine",
    c2: "Others",
    c3: "Secret",
  };
  // svelte-ignore state_referenced_locally
  const STORE = `sjel.projects.${storeKey}`;
  /** Shown, never edited: the server derives these from the path or refuses them. */
  const READ_ONLY = new Set(["project", "open", "type", "moved_to"]);
  const DATES = new Set(["due", "scheduled", "start", "completed"]);
  const statuses = (r: ProjectTask): readonly string[] =>
    r.kind === "task" ? TASK_STATUSES : r.kind === "project" ? PROJECT_STATUSES : ["active", "done", "cancelled"];

  let rows = $state<ProjectTask[]>([]);
  /** Open tasks per project note, for the `open` column of a projects view. */
  let openCount = $state<Map<string, number>>(new Map());
  let loadState = $state<"loading" | "error" | "ready">("loading");
  let error = $state<string | null>(null);
  let saving = $state<string | null>(null);
  const saved = load();
  let query = $state(saved.query ?? "");
  // svelte-ignore state_referenced_locally
  let columns = $state<string[]>(saved.columns ?? initial);
  let groupBy = $state<string>(saved.groupBy ?? "");
  let sort = $state<Sort | null>(saved.sort ?? null);
  let showDone = $state<boolean>(saved.showDone ?? false);
  let layout = $state<LayoutMode>(saved.layout ?? "table");
  let savedViews = $state<SavedView[]>(saved.views ?? []);
  let activeViewName = $state<string>(saved.activeViewName ?? "");
  let newViewName = $state("");
  let savingView = $state(false);

  function load(): {
    columns?: string[];
    groupBy?: string;
    sort?: Sort | null;
    showDone?: boolean;
    layout?: LayoutMode;
    query?: string;
    views?: SavedView[];
    activeViewName?: string;
  } {
    try {
      return JSON.parse(localStorage.getItem(STORE) ?? "{}");
    } catch {
      return {};
    }
  }
  $effect(() => {
    localStorage.setItem(
      STORE,
      JSON.stringify({
        columns,
        groupBy,
        sort,
        showDone,
        layout,
        query,
        views: savedViews,
        activeViewName,
      }),
    );
  });

  function applySavedView(view: SavedView) {
    activeViewName = view.name;
    layout = view.layout;
    columns = [...view.columns];
    groupBy = view.groupBy;
    sort = view.sort ? { ...view.sort } : null;
    showDone = view.showDone;
    query = view.query;
  }

  function saveCurrentView() {
    const name = newViewName.trim();
    if (!name) return;
    const entry: SavedView = {
      name,
      layout,
      columns: [...columns],
      groupBy,
      sort: sort ? { ...sort } : null,
      showDone,
      query,
    };
    const idx = savedViews.findIndex((v) => v.name === name);
    if (idx >= 0) {
      savedViews[idx] = entry;
    } else {
      savedViews = [...savedViews, entry];
    }
    activeViewName = name;
    newViewName = "";
    savingView = false;
  }

  function removeSavedView(name: string) {
    savedViews = savedViews.filter((v) => v.name !== name);
    if (activeViewName === name) activeViewName = "";
  }

  const isDone = (r: ProjectTask) => r.done || ["done", "cancelled"].includes(r.fields.status ?? "");
  const visible = $derived(rows.filter((r) => !filter || filter(r)));

  /** Every key a row in this view carries, the initial columns first. */
  const keys = $derived.by(() => {
    const seen = new Set(["project", ...(kinds.includes("project") ? ["open"] : []), ...initial, "data_class"]);
    for (const r of visible) for (const k of Object.keys(r.fields)) if (k !== "done") seen.add(k);
    return [...seen];
  });
  const shown = $derived(columns.filter((c) => keys.includes(c)));

  const cell = (r: ProjectTask, key: string) =>
    key === "project"
      ? r.project
      : key === "open"
        ? String(openCount.get(r.title) ?? 0)
        : key === "data_class"
          ? r.data_class
          : (r.fields[key] ?? (key === "summary" ? r.summary ?? "" : ""));
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

  const boardKey = $derived(groupBy || "status");
  const boardColumns = $derived.by(() => {
    const key = boardKey;
    const map = new Map<string, ProjectTask[]>();
    if (key === "status") {
      const defaults = kinds.includes("task") ? TASK_STATUSES : PROJECT_STATUSES;
      for (const s of defaults) {
        if (showDone || !["done", "cancelled"].includes(s)) map.set(s, []);
      }
    }
    for (const r of ordered) {
      const name = cell(r, key) || "—";
      map.set(name, [...(map.get(name) ?? []), r]);
    }
    const known: readonly string[] = key === "status" ? [...TASK_STATUSES, ...PROJECT_STATUSES] : [];
    const rank = (n: string) => (known.includes(n) ? known.indexOf(n) : known.length);
    return [...map]
      .map(([name, rows]) => ({ name, rows }))
      .toSorted((a, b) => rank(a.name) - rank(b.name) || a.name.localeCompare(b.name));
  });

  const calendarGroups = $derived.by(() => {
    const map = new Map<string, ProjectTask[]>();
    for (const r of ordered) {
      const date = r.fields.due || r.fields.scheduled || r.fields.start || "Undated";
      map.set(date, [...(map.get(date) ?? []), r]);
    }
    return [...map]
      .map(([date, rows]) => ({ date, rows }))
      .toSorted((a, b) => {
        if (a.date === "Undated") return 1;
        if (b.date === "Undated") return -1;
        return a.date.localeCompare(b.date);
      });
  });

  const rowHref = (r: ProjectTask) =>
    link(`/projects?project=${encodeURIComponent(r.project)}${r.kind === "project" ? "" : `&open=${encodeURIComponent(r.id)}`}`);

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
  <div class="layout-pills" role="group" aria-label="View layout">
    {#each [["table", "Table"], ["board", "Board"], ["list", "List"], ["calendar", "Calendar"]] as [mode, label] (mode)}
      <button
        type="button"
        class="pill-btn"
        class:active={layout === mode}
        onclick={() => (layout = mode as LayoutMode)}
      >
        {label}
      </button>
    {/each}
  </div>
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

<div class="saved-views-bar" aria-label="Saved views">
  {#each savedViews as v (v.name)}
    <span class="saved-view-chip" class:active={activeViewName === v.name}>
      <button type="button" class="saved-view-select" onclick={() => applySavedView(v)}>
        {v.name} <span class="saved-view-meta">({v.layout})</span>
      </button>
      <button
        type="button"
        class="saved-view-remove"
        aria-label={`Remove saved view ${v.name}`}
        use:tip={`Remove view "${v.name}"`}
        onclick={() => removeSavedView(v.name)}
      >
        ×
      </button>
    </span>
  {/each}
  {#if savingView}
    <form
      class="save-view-form"
      onsubmit={(e) => {
        e.preventDefault();
        saveCurrentView();
      }}
    >
      <input
        type="text"
        placeholder="View name…"
        aria-label="Saved view name"
        bind:value={newViewName}
        maxlength="40"
      />
      <button type="submit" class="pill-btn active" disabled={!newViewName.trim()}>Save</button>
      <button type="button" class="pill-btn" onclick={() => (savingView = false)}>Cancel</button>
    </form>
  {:else}
    <button type="button" class="pill-btn save-trigger" onclick={() => (savingView = true)}>
      + Save view
    </button>
  {/if}
</div>

<StateLine state={loadState} message={loadState === "error" ? (error ?? undefined) : "Reading…"} onRetry={reload} />
{#if error && loadState === "ready"}<p class="error" role="alert">{error}</p>{/if}

{#if loadState === "ready"}
  {#if layout === "table"}
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
                <td class="title">
                  <a
                    href={rowHref(r)}
                    onclick={(e) => {
                      if (onselect) {
                        e.preventDefault();
                        onselect(r);
                      }
                    }}
                  >
                    {r.title}
                  </a>
                </td>
                {#each shown as k (k)}
                  {@const label = `${k}: ${r.title}`}
                  <td>
                    {#if READ_ONLY.has(k)}
                      <span class="ro">{cell(r, k)}</span>
                    {:else if k === "data_class"}
                      <select
                        aria-label={label}
                        value={r.data_class}
                        use:tip={r.data_class_rationale}
                        onchange={(e) => set(r, "class", e.currentTarget.value)}
                      >
                        {#each ["c0", "c1", "c2", "c3"] as c (c)}
                          <option value={c}>{CLASS_LABELS[c]} · {c}</option>
                        {/each}
                      </select>
                    {:else if k === "status" && statuses(r).length > 0}
                      <select aria-label={label} value={r.fields.status ?? ""} onchange={(e) => set(r, k, e.currentTarget.value)}>
                        {#if !statuses(r).includes(r.fields.status ?? "")}<option value={r.fields.status ?? ""}>{r.fields.status || "—"}</option>{/if}
                        {#each statuses(r) as s (s)}<option value={s}>{s}</option>{/each}
                      </select>
                    {:else if k === "maturity"}
                      <select aria-label={label} value={r.fields.maturity ?? ""} onchange={(e) => set(r, k, e.currentTarget.value)}>
                        {#if !["seedling", "growing", "mature", "evergreen"].includes(r.fields.maturity ?? "")}
                          <option value={r.fields.maturity ?? ""}>{r.fields.maturity || "—"}</option>
                        {/if}
                        {#each ["seedling", "growing", "mature", "evergreen"] as m (m)}
                          <option value={m}>{m}</option>
                        {/each}
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
                        value={cell(r, k)}
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
  {:else if layout === "board"}
    <div class="board-lanes">
      {#each boardColumns as col (col.name)}
        <div class="board-lane">
          <div class="board-lane-header">
            <span>{col.name}</span>
            <span class="count">{col.rows.length}</span>
          </div>
          <div class="board-cards">
            {#each col.rows as r (r.id)}
              <div class="board-card" class:done={isDone(r)}>
                <div class="board-card-top">
                  <a
                    class="board-card-title"
                    href={rowHref(r)}
                    onclick={(e) => {
                      if (onselect) {
                        e.preventDefault();
                        onselect(r);
                      }
                    }}
                  >
                    {r.title}
                  </a>
                  <span class="class-chip" use:tip={r.data_class_rationale}>{CLASS_LABELS[r.data_class] ?? r.data_class}</span>
                </div>
                <div class="board-card-meta">
                  {#if r.project}<span>{r.project}</span>{/if}
                  {#if r.kind === "task"}<span>P{r.priority}</span>{/if}
                  {#if r.fields.due}<span>due {r.fields.due}</span>{/if}
                </div>
                {#if boardKey === "status" && statuses(r).length > 0}
                  <select
                    class="board-status-select"
                    aria-label={`Status: ${r.title}`}
                    value={r.fields.status ?? (r.done ? "done" : "open")}
                    onchange={(e) => set(r, "status", e.currentTarget.value)}
                  >
                    {#each statuses(r) as s (s)}<option value={s}>{s}</option>{/each}
                  </select>
                {/if}
              </div>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {:else if layout === "list"}
    <ul class="compact-list">
      {#each ordered as r (r.id)}
        <li class="compact-item" class:done={isDone(r)}>
          {#if r.kind === "task"}
            <input
              type="checkbox"
              aria-label={`Done: ${r.title}`}
              checked={r.done}
              onchange={() => set(r, "status", r.done ? "open" : "done")}
            />
          {/if}
          <a
            class="compact-title"
            href={rowHref(r)}
            onclick={(e) => {
              if (onselect) {
                e.preventDefault();
                onselect(r);
              }
            }}
          >
            {r.title}
          </a>
          <span class="compact-meta">
            {#each shown.slice(0, 4) as k (k)}
              {#if cell(r, k)}<span>{k === "data_class" ? (CLASS_LABELS[r.data_class] ?? r.data_class) : cell(r, k)}</span>{/if}
            {/each}
          </span>
        </li>
      {/each}
    </ul>
    {#if ordered.length === 0}<p class="hint">{query ? "Nothing matches." : "Nothing open."}</p>{/if}
  {:else if layout === "calendar"}
    <div class="calendar-buckets">
      {#each calendarGroups as g (g.date)}
        <div class="calendar-bucket">
          <div class="calendar-date">{g.date} <span class="count">{g.rows.length}</span></div>
          <ul class="compact-list">
            {#each g.rows as r (r.id)}
              <li class="compact-item" class:done={isDone(r)}>
                <a
                  class="compact-title"
                  href={rowHref(r)}
                  onclick={(e) => {
                    if (onselect) {
                      e.preventDefault();
                      onselect(r);
                    }
                  }}
                >
                  {r.title}
                </a>
                <span class="compact-meta">
                  <span>{r.project}</span>
                  {#if r.kind === "task"}<span>P{r.priority}</span>{/if}
                  <span class="class-chip" use:tip={r.data_class_rationale}>{CLASS_LABELS[r.data_class] ?? r.data_class}</span>
                </span>
              </li>
            {/each}
          </ul>
        </div>
      {/each}
    </div>
  {/if}
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

  .layout-pills,
  .saved-views-bar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.35rem;
  }

  .saved-views-bar {
    margin-bottom: 0.6rem;
    font-size: var(--text-sm);
  }

  .pill-btn {
    padding: 0.2rem 0.55rem;
    font-size: var(--text-sm);
    color: var(--text-secondary);
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    cursor: pointer;
  }

  .pill-btn.active {
    color: var(--text);
    border-color: var(--accent, var(--text));
    font-weight: 600;
  }

  .saved-view-chip {
    display: inline-flex;
    align-items: center;
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    overflow: hidden;
  }

  .saved-view-chip.active {
    border-color: var(--accent, var(--text));
  }

  .saved-view-select,
  .saved-view-remove {
    all: unset;
    cursor: pointer;
    padding: 0.15rem 0.45rem;
    font-size: var(--text-sm);
  }

  .saved-view-meta {
    color: var(--text-secondary);
  }

  .saved-view-remove {
    color: var(--text-secondary);
    border-left: 1px solid var(--rule);
  }

  .save-view-form {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
  }

  .save-view-form input {
    padding: 0.15rem 0.4rem;
    font-size: var(--text-sm);
    width: 9rem;
  }

  .board-lanes {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: 0.75rem;
    align-items: start;
  }

  .board-lane {
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    padding: 0.55rem;
  }

  .board-lane-header {
    display: flex;
    justify-content: space-between;
    font-size: var(--text-sm);
    font-weight: 600;
    margin-bottom: 0.5rem;
    text-transform: capitalize;
  }

  .board-cards {
    display: grid;
    gap: 0.45rem;
  }

  .board-card {
    padding: 0.5rem;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    background: var(--bg, transparent);
    font-size: var(--text-sm);
    display: grid;
    gap: 0.3rem;
  }

  .board-card.done .board-card-title {
    color: var(--text-secondary);
    text-decoration: line-through;
  }

  .board-card-top {
    display: flex;
    justify-content: space-between;
    gap: 0.4rem;
    align-items: baseline;
  }

  .board-card-title {
    color: inherit;
    text-decoration: none;
    font-weight: 500;
  }

  .board-card-title:hover {
    text-decoration: underline;
  }

  .board-card-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .board-status-select {
    font-size: 0.75rem;
    padding: 0.1rem 0.25rem;
  }

  .class-chip {
    font-size: 0.7rem;
    padding: 0.05rem 0.35rem;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    white-space: nowrap;
  }

  .compact-list {
    list-style: none;
    padding: 0;
    margin: 0;
    border-top: 1px solid var(--rule);
  }

  .compact-item {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.2rem;
    border-bottom: 1px solid var(--rule);
    font-size: var(--text-sm);
  }

  .compact-item.done .compact-title {
    color: var(--text-secondary);
    text-decoration: line-through;
  }

  .compact-title {
    color: inherit;
    text-decoration: none;
    flex: 1 1 auto;
  }

  .compact-title:hover {
    text-decoration: underline;
  }

  .compact-meta {
    display: inline-flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--text-secondary);
    font-size: 0.75rem;
  }

  .calendar-buckets {
    display: grid;
    gap: 0.75rem;
  }

  .calendar-date {
    font-size: var(--text-sm);
    font-weight: 600;
    margin-bottom: 0.25rem;
  }
</style>
