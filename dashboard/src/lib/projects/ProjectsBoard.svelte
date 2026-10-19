<script lang="ts">
  import { ApiError, projects, type ProjectRecord, type ProjectSummary } from "$lib/api";
  import MarkdownDocument from "$lib/feed/MarkdownDocument.svelte";
  import StateLine from "$lib/StateLine.svelte";
  import DatabaseTable from "./DatabaseTable.svelte";
  import { countCheckboxes, fileUrl, resolveFile, segments, setCheckbox } from "./database";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import { focusRow } from "$lib/list-cursor.svelte";

  /**
   * Projects, their tasks and their notes, from `capabilities/projects` (ISA.md F14).
   * The store is canonical; every write here renders the record's markdown before it answers,
   * so the list is reloaded from the server after each write rather than patched in place.
   */
  let { selected, open = null }: { selected: string | null; open?: string | null } = $props();

  let list = $state<ProjectSummary[]>([]);
  let records = $state<ProjectRecord[]>([]);
  let listState = $state<"loading" | "error" | "ready">("loading");
  let recordState = $state<"loading" | "error" | "empty" | "ready">("empty");
  let error = $state<string | null>(null);
  let busy = $state(false);
  let title = $state("");
  let due = $state("");
  let priority = $state(2);
  let noteTitle = $state("");
  /** The record open in the editor: its path and the content the edit started from. */
  let editing = $state<{ path: string; base: string } | null>(null);
  let draft = $state("");
  /** Bumped after each write here, so the tasks table reloads with the list. */
  let stamp = $state(0);

  const live = $derived(list.filter((p) => !p.archived));
  const archived = $derived(list.filter((p) => p.archived));
  const hub = $derived(records.find((r) => r.kind === "project") ?? null);
  const tasks = $derived(
    records
      .filter((r) => r.kind === "task")
      .toSorted((a, b) => Number(a.done) - Number(b.done) || byDue(a.due, b.due) || a.priority - b.priority),
  );
  const notes = $derived(records.filter((r) => r.kind === "note"));
  /** The task named by `?open=`: its body is shown under the table, the way a note opens. */
  const openTask = $derived(tasks.find((t) => t.path === open) ?? null);
  $effect(() => {
    if (open && recordState === "ready") focusRow(document.getElementById(`record-${open}`));
  });
  const canAdd = $derived(selected !== null && !records.some((r) => r.archived));

  /** Undated after dated: a deadline is what expires. Plain comparison, ISO dates sort as text. */
  const byDue = (a: string | null, b: string | null) => (a === b ? 0 : a === null ? 1 : b === null ? -1 : a < b ? -1 : 1);
  /** Markdown for display: no frontmatter, and the Obsidian syntax MarkdownDocument does not know
   *  turned into plain markdown. `segments` split out Bases and attachment embeds before this
   *  runs; a wikilink to an attachment becomes a link to the file, an embed of a note is dropped. */
  const body = (content: string, host: string) =>
    content
      .replace(/^!\[\[.*\]\]\s*$/gm, "")
      // A base embedded inside a callout cannot hold a table; name the view and where it lives.
      .replace(/!\[\[([^\]#|]+?)\.base(?:#([^\]|]+))?(?:\|([^\]]+))?\]\]/g, (_, file: string, view?: string, alias?: string) =>
        `*${alias ?? [file.split("/").pop(), view].filter(Boolean).join(" · ")} (a base in the vault)*`)
      .replace(/!?\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_, target: string, label?: string) => {
        const text = label ?? target.split("/").pop() ?? target;
        const file = target.endsWith(".md") ? null : resolveFile(target, host, files);
        return file ? `[${text}](${fileUrl(file)})` : text;
      })
      .replace(/^> \[!(\w+)\][+-]? ?(.*)$/gm, (_, type: string, title: string) => `> **${title || type[0].toUpperCase() + type.slice(1)}**`);
  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

  /** Every attachment path, read once: wikilinks resolve against it. */
  let files = $state<string[]>([]);
  $effect(() => {
    projects.files().then((f) => (files = f), () => {});
  });

  async function loadList() {
    try {
      list = await projects.list();
      listState = "ready";
    } catch (e) {
      error = message(e);
      listState = "error";
    }
  }

  async function loadRecords(name: string | null) {
    if (!name) {
      records = [];
      recordState = "empty";
      return;
    }
    recordState = "loading";
    try {
      records = await projects.get(name);
      recordState = "ready";
    } catch (e) {
      error = message(e);
      recordState = "error";
    }
  }

  /** Each segment with the number of the first checkbox in it, so a box in the third markdown run
   *  of a note maps back to its line in the whole record. */
  const numbered = (parts: ReturnType<typeof segments>) => {
    let next = 0;
    return parts.map((part) => {
      const first = next;
      if ("markdown" in part) next += countCheckboxes(part.markdown);
      return { part, first };
    });
  };

  /** Ticks one `- [ ]` in a record and saves it, with the shown content as the base. */
  function toggle(record: ProjectRecord, box: number, checked: boolean) {
    const next = setCheckbox(record.content, box, checked);
    if (next !== null) void write(() => projects.save(record.path, next, record.content));
  }

  async function write(run: () => Promise<unknown>) {
    busy = true;
    error = null;
    try {
      await run();
      stamp++;
      await Promise.all([loadList(), loadRecords(selected)]);
    } catch (e) {
      error = message(e);
    } finally {
      busy = false;
    }
  }

  function addTask(event: SubmitEvent) {
    event.preventDefault();
    if (!selected || !title.trim()) return;
    const project = selected;
    void write(async () => {
      await projects.addTask({ project, title: title.trim(), due: due || undefined, priority });
      title = "";
      due = "";
    });
  }

  function addNote(event: SubmitEvent) {
    event.preventDefault();
    if (!selected || !noteTitle.trim()) return;
    const project = selected;
    void write(async () => {
      await projects.addNote(project, noteTitle.trim());
      noteTitle = "";
    });
  }

  async function saveEdit() {
    if (!editing) return;
    const { path, base } = editing;
    busy = true;
    error = null;
    try {
      await projects.save(path, draft, base);
      editing = null;
      await Promise.all([loadList(), loadRecords(selected)]);
    } catch (e) {
      // A stale base keeps the draft open, so nothing typed is lost; the list reloads underneath.
      error =
        e instanceof ApiError && e.status === 409
          ? "This note changed since you opened it. Nothing was saved. Copy your draft, cancel, and edit again."
          : message(e);
      await loadRecords(selected);
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    void loadList();
  });
  $effect(() => {
    void loadRecords(selected);
  });
</script>

<div class="board">
  <nav aria-label="Projects">
    <StateLine state={listState} message={listState === "error" ? (error ?? undefined) : "Reading projects…"} onRetry={loadList} />
    <ul>
      {#each live as p (p.name)}
        <li>
          <a href={link(`/projects?project=${encodeURIComponent(p.name)}`)} aria-current={p.name === selected ? "page" : undefined}>
            <span class="name" use:tip={p.name}>{p.name}</span>
            {#if p.open > 0}<span class="count">{p.open}</span>{/if}
          </a>
        </li>
      {/each}
    </ul>
    {#if archived.length > 0}
      <details>
        <summary>Archived · {archived.length}</summary>
        <ul>
          {#each archived as p (p.name)}
            <li>
              <a href={link(`/projects?project=${encodeURIComponent(p.name)}`)} aria-current={p.name === selected ? "page" : undefined}>
                <span class="name" use:tip={p.name}>{p.name}</span>
              </a>
            </li>
          {/each}
        </ul>
      </details>
    {/if}
  </nav>

  <section aria-live="polite">
    {#if !selected}
      <h2>Projects</h2>
      <DatabaseTable kinds={["project"]} columns={["status", "open", "priority", "start", "due"]} storeKey="projects" {stamp} />
      <h2>All tasks</h2>
      <DatabaseTable
        kinds={["task"]}
        columns={["project", "status", "due", "scheduled", "priority", "context", "energy"]}
        storeKey="tasks"
        {stamp}
        onchange={() => {
          stamp++;
          void loadList();
        }}
      />
    {:else}
      <StateLine state={recordState} message={recordState === "error" ? (error ?? undefined) : "Reading the project…"} onRetry={() => loadRecords(selected)} />
      {#if recordState === "ready"}
        {#if error}<p class="error" role="alert">{error}</p>{/if}

        <h2>Tasks <span class="count">{tasks.filter((t) => !t.done).length} open</span></h2>
        {#if canAdd}
          {@const project = selected}
          <DatabaseTable
            kinds={["task"]}
            filter={(r) => r.project === project}
            columns={["status", "due", "scheduled", "priority", "context", "energy"]}
            storeKey="project-tasks"
            {stamp}
            onchange={() => Promise.all([loadList(), loadRecords(selected)])}
          />
        {:else}
        <ul class="tasks">
          {#each tasks as t (t.path)}
            <li class:done={t.done}>
              <label>
                <input
                  type="checkbox"
                  checked={t.done}
                  disabled={busy || t.archived}
                  onchange={() => write(() => projects.setDone(t.path, !t.done))}
                />
                <span class="title">{t.title}</span>
              </label>
              <span class="meta">P{t.priority}{t.due ? ` · due ${t.due}` : ""}</span>
            </li>
          {/each}
        </ul>
        {/if}

        {#if openTask}
          <article class="opened" id={`record-${openTask.path}`} tabindex="-1">
            <h3>{openTask.title}</h3>
            {@render editable(openTask)}
            <a class="close" href={link(`/projects?project=${encodeURIComponent(selected ?? "")}`)}>Close</a>
          </article>
        {/if}

        {#if canAdd}
          <form onsubmit={addTask}>
            <input aria-label="New task" placeholder="New task" bind:value={title} maxlength="200" required />
            <input aria-label="Due date" type="date" bind:value={due} />
            <select aria-label="Priority" bind:value={priority}>
              <option value={1}>P1</option>
              <option value={2}>P2</option>
              <option value={3}>P3</option>
            </select>
            <button type="submit" class="btn" disabled={busy || !title.trim()}>Add</button>
          </form>
        {/if}

        {#if hub}
          <h2>Overview</h2>
          {@render editable(hub)}
        {/if}

        <h2>Notes <span class="count">{notes.length}</span></h2>
        {#each notes as n (n.path)}
          <details class="note" id={`record-${n.path}`} open={editing?.path === n.path || open === n.path}>
            <summary>{n.title}</summary>
            {@render editable(n)}
          </details>
        {/each}
        {#if canAdd}
          <form onsubmit={addNote}>
            <input aria-label="New note" placeholder="New note" bind:value={noteTitle} maxlength="200" required />
            <button type="submit" class="btn" disabled={busy || !noteTitle.trim()}>Add note</button>
          </form>
        {/if}
      {/if}
    {/if}
  </section>
</div>

{#snippet editable(record: ProjectRecord)}
  {#if editing && editing.path === record.path}
    <textarea aria-label={`Edit ${record.title}`} bind:value={draft} rows="18" spellcheck="false"></textarea>
    <div class="edit-actions">
      <button type="button" class="btn" disabled={busy || draft === editing.base} onclick={saveEdit}>Save</button>
      <button type="button" class="btn btn-soft" disabled={busy} onclick={() => (editing = null)}>Cancel</button>
    </div>
  {:else}
    {#each numbered(segments(record.content.replace(/^---\n[\s\S]*?\n---\n/, ""), record.path, files)) as { part, first }, i (i)}
      {#if "markdown" in part}
        <MarkdownDocument
          content={body(part.markdown, record.path)}
          compact
          resolveLink={(href) => href}
          onToggle={busy || record.archived ? undefined : (box, checked) => toggle(record, first + box, checked)}
        />
      {:else if "file" in part}
        {#if part.image}
          <img class="attachment" src={fileUrl(part.file)} alt={part.file.split("/").pop()} loading="lazy" />
        {:else}
          <p class="vault-base"><a href={fileUrl(part.file)} target="_blank" rel="noreferrer">{part.file.split("/").pop()}</a></p>
        {/if}
      {:else if "vault" in part.embed}
        <p class="vault-base">
          {part.embed.label} ·
          <a href={`obsidian://open?vault=Knowledge-Base&file=${encodeURIComponent(part.embed.vault)}`}>open the base in Obsidian</a>
          <span class="count">vault data, not in Sjel</span>
        </p>
      {:else}
        <div class="embed">
          <h3>{part.embed.label}</h3>
          <DatabaseTable
            kinds={part.embed.kinds}
            filter={part.embed.filter}
            columns={part.embed.columns}
            storeKey={`embed:${record.path}:${i}`}
            {stamp}
            onchange={() => Promise.all([loadList(), loadRecords(selected)])}
          />
        </div>
      {/if}
    {/each}
    <button
      type="button"
      class="btn btn-soft edit"
      disabled={busy || editing !== null}
      onclick={() => {
        editing = { path: record.path, base: record.content };
        draft = record.content;
      }}>Edit</button
    >
  {/if}
{/snippet}

<style>
  textarea {
    width: 100%;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    background: var(--input-bg);
    border: 1px solid var(--input-border);
    border-radius: var(--radius-sm);
    padding: 0.6rem;
  }

  .edit-actions {
    display: flex;
    gap: 0.5rem;
    margin: 0.4rem 0 0.8rem;
  }

  .edit {
    margin: 0.3rem 0 0.8rem;
  }

  .board {
    display: grid;
    grid-template-columns: minmax(12rem, 16rem) 1fr;
    gap: 1.5rem;
    align-items: start;
  }

  @media (max-width: 720px) {
    .board {
      grid-template-columns: 1fr;
    }
  }

  nav ul,
  .tasks {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  nav a {
    display: flex;
    height: auto;
    line-height: 1.3;
    justify-content: space-between;
    gap: 0.5rem;
    padding: 0.35rem 0.5rem;
    border-radius: var(--radius-sm);
    color: inherit;
    text-decoration: none;
  }

  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  nav a[aria-current="page"] {
    background: var(--primary-soft);
    color: var(--primary);
    font-weight: 600;
  }

  details summary {
    cursor: pointer;
    padding: 0.35rem 0.5rem;
    color: var(--text-secondary);
  }

  h2 {
    margin: 1.25rem 0 0.5rem;
    font-size: var(--text-md);
  }

  .count,
  .meta {
    color: var(--text-secondary);
    font-size: var(--text-sm);
    font-weight: 400;
  }

  .tasks li {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.4rem 0;
    border-bottom: 1px solid var(--rule);
  }

  .tasks label {
    display: flex;
    gap: 0.6rem;
    align-items: baseline;
  }

  .tasks li.done .title {
    color: var(--text-secondary);
    text-decoration: line-through;
  }

  form {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }

  form input:first-child {
    flex: 1 1 14rem;
  }

  .embed {
    margin: 0.75rem 0 1rem;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
  }

  .embed h3 {
    margin: 0 0 0.4rem;
    font-size: var(--text-sm);
    color: var(--text-secondary);
  }

  .attachment {
    display: block;
    max-width: 100%;
    max-height: 32rem;
    margin: 0.5rem 0;
    border-radius: var(--radius-sm);
  }

  .vault-base {
    margin: 0.5rem 0;
    font-size: var(--text-sm);
  }

  .note {
    border-bottom: 1px solid var(--rule);
  }

  .opened {
    border-top: 1px solid var(--rule);
    border-bottom: 1px solid var(--rule);
    margin: 0.8rem 0;
  }

  .error {
    color: var(--danger);
  }
</style>
