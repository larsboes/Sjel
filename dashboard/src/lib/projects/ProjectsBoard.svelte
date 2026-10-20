<script lang="ts">
  import { ApiError, projects, type ProjectRecord, type ProjectSummary } from "$lib/api";
  import MarkdownDocument from "$lib/feed/MarkdownDocument.svelte";
  import StateLine from "$lib/StateLine.svelte";
  import Icon from "$lib/Icon.svelte";
  import DatabaseTable from "./DatabaseTable.svelte";
  import ProjectsGraph from "./ProjectsGraph.svelte";
  import {
    countCheckboxes,
    extractExcerpt,
    extractFrontmatter,
    fileUrl,
    findBacklinks,
    recordProject,
    recordTitle,
    resolveFile,
    resolveRecord,
    segments,
    setCheckbox,
  } from "./database";
  import { figures } from "$lib/figures";
  import GenUIRenderer from "$lib/genui/GenUIRenderer.svelte";
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import { focusRow } from "$lib/list-cursor.svelte";

  /**
   * Projects, their tasks and their notes, from `capabilities/projects` (its README).
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

  // New project creation state
  let addingProject = $state(false);
  let newProjectName = $state("");
  let newProjectSummary = $state("");

  // Task & note creation state
  let title = $state("");
  let due = $state("");
  let priority = $state(2);
  let noteTitle = $state("");

  // Notes view mode (cards vs database table vs graph)
  let notesView = $state<"cards" | "table" | "graph">("cards");
  // In-editor live preview toggle
  let editorPreview = $state(false);

  /** The record open in the editor: its path and the content the edit started from. */
  let editing = $state<{ path: string; base: string } | null>(null);
  let draft = $state("");
  /** Bumped after each write here, so the tasks table reloads with the list. */
  let stamp = $state(0);

  // Hover preview popover state
  let hoverPreview = $state<{
    x: number;
    y: number;
    title: string;
    kind: string;
    excerpt: string;
    due?: string | null;
    priority?: number;
    path: string;
    project?: string;
    maturity?: string;
    status?: string;
    backlinksCount?: number;
    visible: boolean;
  } | null>(null);
  let hoverTimer: ReturnType<typeof setTimeout> | null = null;

  const live = $derived(list.filter((p) => !p.archived));
  const archived = $derived(list.filter((p) => p.archived));
  const hub = $derived(records.find((r) => r.kind === "project") ?? null);
  const tasks = $derived(
    records
      .filter((r) => r.kind === "task")
      .toSorted((a, b) => Number(a.done) - Number(b.done) || byDue(a.due, b.due) || a.priority - b.priority),
  );
  const notes = $derived(records.filter((r) => r.kind === "note"));
  /** The task or note named by `?open=`: its body is shown under the table, the way a note opens. */
  const openTask = $derived(tasks.find((t) => t.path === open) ?? null);
  const openNote = $derived(notes.find((n) => n.path === open) ?? null);
  $effect(() => {
    if (open && recordState === "ready") focusRow(document.getElementById(`record-${open}`));
  });
  const canAdd = $derived(selected !== null && !records.some((r) => r.archived));

  /** Undated after dated: a deadline is what expires. Plain comparison, ISO dates sort as text. */
  const byDue = (a: string | null, b: string | null) => (a === b ? 0 : a === null ? 1 : b === null ? -1 : a < b ? -1 : 1);

  /** Markdown for display: no frontmatter, and the Obsidian syntax MarkdownDocument does not know
   *  turned into plain markdown. Resolves wikilinks against attachments, notes in the current project,
   *  and other projects across Sjel. */
  const body = (content: string, host: string) =>
    content
      .replace(/^!\[\[.*\]\]\s*$/gm, "")
      // A base embedded inside a callout cannot hold a table; name the view and where it lives.
      .replace(/!\[\[([^\]#|]+?)\.base(?:#([^\]|]+))?(?:\|([^\]]+))?\]\]/g, (_, file: string, view?: string, alias?: string) =>
        `*${alias ?? [file.split("/").pop(), view].filter(Boolean).join(" · ")} (a base in the vault)*`)
      .replace(/!?\[\[([^\]|#]+)(?:#([^\]|]+))?(?:\|([^\]]+))?\]\]/g, (_, target: string, _anchor?: string, label?: string) => {
        const text = label ?? target.split("/").pop() ?? target;
        // 1. Check attachment
        const file = target.endsWith(".md") ? null : resolveFile(target, host, files);
        if (file) return `[${text}](${fileUrl(file)})`;
        // 2. Check current project records
        const matched = resolveRecord(target, host, records);
        if (matched) {
          const proj = recordProject(matched.path);
          return `[${text}](/projects?project=${encodeURIComponent(proj)}&open=${encodeURIComponent(matched.path)})`;
        }
        // 3. Check other projects by name
        const cleanTarget = target.split("/").pop()?.trim().replace(/\.md$/, "");
        const projMatch = list.find((p) => p.name === cleanTarget);
        if (projMatch) {
          return `[${text}](/projects?project=${encodeURIComponent(projMatch.name)})`;
        }
        return text;
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

  async function handleAddProject(event: SubmitEvent) {
    event.preventDefault();
    if (!newProjectName.trim()) return;
    const name = newProjectName.trim();
    const summary = newProjectSummary.trim();
    void write(async () => {
      await projects.addProject(name, summary);
      newProjectName = "";
      newProjectSummary = "";
      addingProject = false;
    });
  }

  async function deleteRecord(path: string, itemTitle: string) {
    if (!confirm(`Delete "${itemTitle}"? This cannot be undone.`)) return;
    void write(async () => {
      await projects.remove(path);
      if (open === path && typeof window !== "undefined") {
        window.location.href = link(`/projects?project=${encodeURIComponent(selected ?? "")}`);
      }
    });
  }

  async function toggleArchiveProject(hubRecord: ProjectRecord) {
    const next = !hubRecord.archived;
    void write(async () => {
      await projects.setProperty(hubRecord.path, "archived", next ? "true" : "");
    });
  }

  async function deleteProject(projectName: string) {
    if (!confirm(`Permanently delete project "${projectName}" and all its tasks and notes?`)) return;
    void write(async () => {
      const toDelete = records.filter((r) => recordProject(r.path) === projectName || r.path.startsWith(`${projectName}/`));
      for (const r of toDelete) {
        await projects.remove(r.path);
      }
      if (typeof window !== "undefined") {
        window.location.href = link("/projects");
      }
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
      error =
        e instanceof ApiError && e.status === 409
          ? "This note changed since you opened it. Nothing was saved. Copy your draft, cancel, and edit again."
          : message(e);
      await loadRecords(selected);
    } finally {
      busy = false;
    }
  }

  function insertGenUI(template: "metrics" | "matrix" | "simulator" | "timeline") {
    let snippet = "";
    if (template === "metrics") {
      snippet = `\n\`\`\`gen-ui\n{\n  "type": "metric-grid",\n  "title": "Subsystem Telemetry",\n  "columns": 3,\n  "items": [\n    { "label": "Mains Line (L-N)", "value": "231.8", "unit": "V", "status": "nominal" },\n    { "label": "Filter Resistance", "value": "1.05", "unit": "MΩ", "status": "good" },\n    { "label": "Leakage Current", "value": "< 0.5", "unit": "mA", "status": "good" }\n  ]\n}\n\`\`\`\n`;
    } else if (template === "matrix") {
      snippet = `\n\`\`\`gen-ui\n{\n  "type": "decision-matrix",\n  "title": "Component Trade-Off Matrix",\n  "description": "Adjust criteria weights and option scores to evaluate trade-offs.",\n  "criteria": [\n    { "id": "cost", "name": "Cost", "weight": 4 },\n    { "id": "reliability", "name": "Reliability", "weight": 5 },\n    { "id": "leadtime", "name": "Lead Time", "weight": 3 }\n  ],\n  "options": [\n    {\n      "id": "opt-1",\n      "name": "Original Miele Filter (4364110)",\n      "tag": "Factory OEM",\n      "scores": { "cost": 6, "reliability": 10, "leadtime": 8 }\n    },\n    {\n      "id": "opt-2",\n      "name": "Generic Replacement (Miflex X1/Y2)",\n      "tag": "Budget Option",\n      "scores": { "cost": 9, "reliability": 7, "leadtime": 9 }\n    }\n  ]\n}\n\`\`\`\n`;
    } else if (template === "simulator") {
      snippet = `\n\`\`\`gen-ui\n{\n  "type": "simulator",\n  "title": "Cycle Energy & Cost Estimator",\n  "description": "Adjust load weight and temperature to calculate cycle requirements.",\n  "inputs": [\n    { "id": "load", "label": "Wash Load", "type": "slider", "min": 1, "max": 6, "step": 1, "default": 4, "unit": "kg" },\n    { "id": "temp", "label": "Temperature", "type": "segmented", "default": 40, "options": [{ "label": "30°C", "value": 30 }, { "label": "40°C", "value": 40 }, { "label": "60°C", "value": 60 }] }\n  ],\n  "outputs": [\n    { "id": "energy", "label": "Energy Required", "unit": "kWh", "formula": "load * temp * 0.0045 + 0.12", "decimals": 2 },\n    { "id": "cost", "label": "Estimated Cost", "format": "currency", "formula": "(load * temp * 0.0045 + 0.12) * 0.38", "decimals": 2 }\n  ]\n}\n\`\`\`\n`;
    } else if (template === "timeline") {
      snippet = `\n\`\`\`gen-ui\n{\n  "type": "status-timeline",\n  "title": "Repair & Verification Milestones",\n  "steps": [\n    { "label": "Initial Fault Diagnosis", "date": "2026-10-18", "status": "done", "detail": "RCD trip isolated to mains filter 4364110." },\n    { "label": "Bench Test & Safety Checks", "date": "2026-10-19", "status": "done", "detail": "M1–M5 insulation tests performed." },\n    { "label": "Filter Replacement Installation", "status": "current", "detail": "Install replacement filter with correct crimp lugs." },\n    { "label": "Full Cycle Verification Run", "status": "pending", "detail": "Test 60°C program with live power monitoring." }\n  ]\n}\n\`\`\`\n`;
    }
    draft = draft ? `${draft.trimEnd()}\n${snippet}` : snippet;
  }

  function handlePointerOver(e: PointerEvent) {
    const target = (e.target as HTMLElement)?.closest("a");
    if (!target) return;
    const href = target.getAttribute("href");
    if (!href) return;

    let targetPath: string | null = null;
    let projName: string | null = null;

    try {
      const url = new URL(href, window.location.origin);
      targetPath = url.searchParams.get("open");
      projName = url.searchParams.get("project");
    } catch {
      // not a standard link
    }

    if (!targetPath && !projName) return;

    if (hoverTimer) clearTimeout(hoverTimer);

    let rec: ProjectRecord | undefined;
    if (targetPath) {
      rec = records.find((r) => r.path === targetPath || recordTitle(r.path) === recordTitle(targetPath!));
    } else if (projName) {
      rec = records.find((r) => r.kind === "project" && recordProject(r.path) === projName);
    }

    if (rec) {
      const rect = target.getBoundingClientRect();
      const excerpt = extractExcerpt(rec.content, 14, '\n\n');
      const fm = extractFrontmatter(rec.content);
      const bl = findBacklinks(rec, records);
      const cardWidth = 360;
      const cardHeight = 240;
      const isBottom = rect.bottom + cardHeight + 16 > window.innerHeight;
      hoverPreview = {
        x: Math.max(12, Math.min(rect.left, window.innerWidth - cardWidth - 16)),
        y: isBottom ? Math.max(12, rect.top - cardHeight - 8) : rect.bottom + 8,
        title: rec.title,
        kind: rec.kind,
        excerpt,
        due: rec.due,
        priority: rec.priority,
        path: rec.path,
        project: recordProject(rec.path),
        maturity: fm.maturity,
        status: fm.status,
        backlinksCount: bl.length,
        visible: true,
      };
    } else if (projName) {
      const proj = list.find((p) => p.name === projName);
      if (proj) {
        const rect = target.getBoundingClientRect();
        const cardWidth = 340;
        const cardHeight = 160;
        const isBottom = rect.bottom + cardHeight + 16 > window.innerHeight;
        hoverPreview = {
          x: Math.max(12, Math.min(rect.left, window.innerWidth - cardWidth - 16)),
          y: isBottom ? Math.max(12, rect.top - cardHeight - 8) : rect.bottom + 8,
          title: proj.name,
          kind: "project",
          excerpt: `${proj.open} open task(s) · ${proj.notes} note(s)`,
          path: proj.name,
          project: proj.name,
          visible: true,
        };
      }
    }
  }

  function handlePointerOut(e: PointerEvent) {
    const target = (e.target as HTMLElement)?.closest("a");
    if (!target) return;
    if (hoverTimer) clearTimeout(hoverTimer);
    hoverTimer = setTimeout(() => {
      if (hoverPreview) hoverPreview.visible = false;
    }, 280);
  }

  $effect(() => {
    void loadList();
  });
  $effect(() => {
    void loadRecords(selected);
  });
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="board" onpointerover={handlePointerOver} onpointerout={handlePointerOut}>
  <!-- Hover Preview Popover -->
  {#if hoverPreview && hoverPreview.visible}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="hover-preview-card"
      style="left: {hoverPreview.x}px; top: {hoverPreview.y}px;"
      role="tooltip"
      tabindex="-1"
      onpointerenter={() => {
        if (hoverTimer) clearTimeout(hoverTimer);
      }}
      onpointerleave={() => {
        if (hoverTimer) clearTimeout(hoverTimer);
        hoverTimer = setTimeout(() => {
          if (hoverPreview) hoverPreview.visible = false;
        }, 250);
      }}
    >
      <div class="hover-preview-header">
        <span class="hover-preview-badge">{hoverPreview.kind}</span>
        {#if hoverPreview.project}<span class="hover-preview-proj">{hoverPreview.project}</span>{/if}
        {#if hoverPreview.priority}<span class="hover-preview-prio">P{hoverPreview.priority}</span>{/if}
        {#if hoverPreview.due}<span class="hover-preview-due">due {hoverPreview.due}</span>{/if}
        {#if hoverPreview.maturity}<span class="hover-preview-mat">{hoverPreview.maturity}</span>{/if}
        {#if hoverPreview.status}<span class="hover-preview-status">{hoverPreview.status}</span>{/if}
      </div>
      <div class="hover-preview-title">
        <a
          href={link(hoverPreview.kind === "project" ? `/projects?project=${encodeURIComponent(hoverPreview.title)}` : `/projects?project=${encodeURIComponent(hoverPreview.project ?? "")}&open=${encodeURIComponent(hoverPreview.path)}`)}
          onclick={() => {
            if (hoverPreview) hoverPreview.visible = false;
          }}
        >
          {hoverPreview.title}
        </a>
      </div>
      {#if hoverPreview.excerpt}
        <div class="hover-preview-scroll-area">
          <div class="hover-preview-excerpt">{hoverPreview.excerpt}</div>
        </div>
      {/if}
      <div class="hover-preview-footer">
        {#if hoverPreview.backlinksCount !== undefined && hoverPreview.backlinksCount > 0}
          <span class="hover-preview-bl">{hoverPreview.backlinksCount} backlink{hoverPreview.backlinksCount === 1 ? "" : "s"}</span>
        {:else}
          <span></span>
        {/if}
        <a
          class="hover-preview-open-link"
          href={link(hoverPreview.kind === "project" ? `/projects?project=${encodeURIComponent(hoverPreview.title)}` : `/projects?project=${encodeURIComponent(hoverPreview.project ?? "")}&open=${encodeURIComponent(hoverPreview.path)}`)}
          onclick={() => {
            if (hoverPreview) hoverPreview.visible = false;
          }}
        >
          Open {hoverPreview.kind} ↗
        </a>
      </div>
    </div>
  {/if}

  <nav aria-label="Projects">
    <div class="nav-header">
      <span class="nav-title">Projects</span>
      <button
        type="button"
        class="btn-new-project"
        use:tip={"Add new project"}
        onclick={() => (addingProject = !addingProject)}
      >
        + New
      </button>
    </div>

    {#if addingProject}
      <form class="new-project-form" onsubmit={handleAddProject}>
        <input
          aria-label="New project name"
          placeholder="Project name…"
          bind:value={newProjectName}
          maxlength="80"
          required
        />
        <input
          aria-label="Project summary"
          placeholder="Summary (optional)…"
          bind:value={newProjectSummary}
          maxlength="200"
        />
        <div class="form-row-btns">
          <button type="submit" class="btn btn-sm" disabled={busy || !newProjectName.trim()}>Create</button>
          <button type="button" class="btn btn-soft btn-sm" onclick={() => (addingProject = false)}>Cancel</button>
        </div>
      </form>
    {/if}

    <StateLine state={listState} message={listState === "error" ? (error ?? undefined) : "Reading projects…"} onRetry={loadList} />
    <ul>
      {#each live as p (p.name)}
        <li>
          <a href={link(`/projects?project=${encodeURIComponent(p.name)}`)} aria-current={p.name === selected ? "page" : undefined}>
            <span class="name" use:tip={p.name}>{p.name}</span>
            <div class="counts">
              {#if p.notes > 0}<span class="notes-count" use:tip={`${p.notes} note(s)`}><Icon name="file-text" size={12} /> {p.notes}</span>{/if}
              {#if p.open > 0}<span class="count" use:tip={`${p.open} open task(s)`}>{p.open}</span>{/if}
            </div>
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
      <DatabaseTable kinds={["project"]} columns={["status", "open", "priority", "start", "due", "class"]} storeKey="projects" {stamp} />
      <h2>All tasks</h2>
      <DatabaseTable
        kinds={["task"]}
        columns={["project", "status", "due", "scheduled", "priority", "context", "energy", "class"]}
        storeKey="tasks"
        {stamp}
        onchange={() => {
          stamp++;
          void loadList();
        }}
      />
      <h2>All notes</h2>
      <DatabaseTable
        kinds={["note"]}
        columns={["project", "summary", "maturity", "type", "categories", "class"]}
        storeKey="all-notes"
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

        <div class="project-toolbar">
          <div class="project-heading-cluster">
            <h2>{selected}</h2>
            {#if hub?.archived}<span class="archived-pill">Archived</span>{/if}
          </div>
          <div class="project-actions">
            {#if hub}
              <button
                type="button"
                class="btn btn-soft btn-sm"
                disabled={busy}
                onclick={() => toggleArchiveProject(hub)}
              >
                {hub.archived ? "Unarchive" : "Archive"}
              </button>
            {/if}
            <button
              type="button"
              class="btn btn-danger-soft btn-sm"
              disabled={busy}
              onclick={() => selected && deleteProject(selected)}
            >
              Delete Project
            </button>
          </div>
        </div>

        <h3>Tasks <span class="count">{tasks.filter((t) => !t.done).length} open</span></h3>
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
              <div class="task-right-meta">
                <span class="meta">P{t.priority}{t.due ? ` · due ${t.due}` : ""}</span>
                <button
                  type="button"
                  class="btn-icon-subtle"
                  use:tip={"Delete task"}
                  onclick={() => deleteRecord(t.path, t.title)}
                >
                  ✕
                </button>
              </div>
            </li>
          {/each}
        </ul>
        {/if}

        {#if openTask}
          {@const taskBacklinks = findBacklinks(openTask, records)}
          <article class="opened" id={`record-${openTask.path}`} tabindex="-1">
            <div class="opened-header">
              <div class="opened-title-wrap">
                <span class="opened-badge">Task</span>
                <h3>{openTask.title}</h3>
              </div>
              <div class="opened-actions">
                <button
                  type="button"
                  class="btn btn-danger-soft btn-sm"
                  onclick={() => deleteRecord(openTask.path, openTask.title)}
                >
                  Delete
                </button>
                <a class="close btn btn-sm" href={link(`/projects?project=${encodeURIComponent(selected ?? "")}`)}>Close</a>
              </div>
            </div>
            {@render editable(openTask)}
            {#if taskBacklinks.length > 0}
              <div class="backlinks-section">
                <span class="backlinks-label">Referenced by ({taskBacklinks.length}):</span>
                <div class="backlinks-list">
                  {#each taskBacklinks as bl (bl.path)}
                    {@const proj = recordProject(bl.path)}
                    <a
                      href={link(`/projects?project=${encodeURIComponent(proj)}&open=${encodeURIComponent(bl.path)}`)}
                      class="backlink-chip"
                    >
                      <span class="chip-kind">{bl.kind}</span>
                      <span class="chip-title">{bl.title}</span>
                    </a>
                  {/each}
                </div>
              </div>
            {/if}
          </article>
        {/if}

        {#if canAdd}
          <form class="add-task-form" onsubmit={addTask}>
            <input aria-label="New task" placeholder="New task…" bind:value={title} maxlength="200" required />
            <input aria-label="Due date" type="date" bind:value={due} />
            <select aria-label="Priority" bind:value={priority}>
              <option value={1}>P1 (High)</option>
              <option value={2}>P2 (Medium)</option>
              <option value={3}>P3 (Low)</option>
            </select>
            <button type="submit" class="btn" disabled={busy || !title.trim()}>+ Add task</button>
          </form>
        {/if}

        {#if hub}
          {@const hubBacklinks = findBacklinks(hub, records)}
          <h3>Overview</h3>
          <div class="hub-container">
            {@render editable(hub)}
            {#if hubBacklinks.length > 0}
              <div class="backlinks-section">
                <span class="backlinks-label">Referenced by ({hubBacklinks.length}):</span>
                <div class="backlinks-list">
                  {#each hubBacklinks as bl (bl.path)}
                    {@const proj = recordProject(bl.path)}
                    <a
                      href={link(`/projects?project=${encodeURIComponent(proj)}&open=${encodeURIComponent(bl.path)}`)}
                      class="backlink-chip"
                    >
                      <span class="chip-kind">{bl.kind}</span>
                      <span class="chip-title">{bl.title}</span>
                    </a>
                  {/each}
                </div>
              </div>
            {/if}
          </div>
        {/if}

        <div class="notes-header-bar">
          <h3>Notes <span class="count">{notes.length}</span></h3>
          <div class="notes-view-switcher">
            <button
              type="button"
              class="view-switch-btn"
              class:active={notesView === "cards"}
              onclick={() => (notesView = "cards")}
              use:tip={"Cards view"}
            >
              <Icon name="layout" size={13} /> Cards
            </button>
            <button
              type="button"
              class="view-switch-btn"
              class:active={notesView === "table"}
              onclick={() => (notesView = "table")}
              use:tip={"Database table view"}
            >
              <Icon name="database" size={13} /> Database Table
            </button>
            <button
              type="button"
              class="view-switch-btn"
              class:active={notesView === "graph"}
              onclick={() => (notesView = "graph")}
              use:tip={"Interactive knowledge & link graph"}
            >
              <Icon name="git-branch" size={13} /> Graph
            </button>
          </div>
        </div>

        {#if notesView === "graph"}
          <ProjectsGraph {records} />
        {:else if notesView === "table"}
          {@const project = selected}
          <div class="notes-table-wrap">
            <DatabaseTable
              kinds={["note"]}
              filter={(r) => r.project === project}
              columns={["summary", "maturity", "type", "categories", "status"]}
              storeKey="project-notes"
              {stamp}
              onchange={() => Promise.all([loadList(), loadRecords(selected)])}
            />
          </div>

          {#if openNote}
            {@const noteBacklinks = findBacklinks(openNote, records)}
            <article class="opened" id={`record-${openNote.path}`} tabindex="-1">
              <div class="opened-header">
                <div class="opened-title-wrap">
                  <span class="opened-badge">Note</span>
                  <h3>{openNote.title}</h3>
                </div>
                <div class="opened-actions">
                  <button
                    type="button"
                    class="btn btn-danger-soft btn-sm"
                    onclick={() => deleteRecord(openNote.path, openNote.title)}
                  >
                    Delete
                  </button>
                  <a class="close btn btn-sm" href={link(`/projects?project=${encodeURIComponent(selected ?? "")}`)}>Close</a>
                </div>
              </div>
              {@render editable(openNote)}
              {#if noteBacklinks.length > 0}
                <div class="backlinks-section">
                  <span class="backlinks-label">Referenced by ({noteBacklinks.length}):</span>
                  <div class="backlinks-list">
                    {#each noteBacklinks as bl (bl.path)}
                      {@const proj = recordProject(bl.path)}
                      <a
                        href={link(`/projects?project=${encodeURIComponent(proj)}&open=${encodeURIComponent(bl.path)}`)}
                        class="backlink-chip"
                      >
                        <span class="chip-kind">{bl.kind}</span>
                        <span class="chip-title">{bl.title}</span>
                      </a>
                    {/each}
                  </div>
                </div>
              {/if}
            </article>
          {/if}
        {:else}
          <div class="notes-stack">
            {#each notes as n (n.path)}
              {@const noteBacklinks = findBacklinks(n, records)}
              <details class="note" id={`record-${n.path}`} open={editing?.path === n.path || open === n.path}>
                <summary>
                  <div class="note-summary-main">
                    <span class="note-icon" aria-hidden="true"><Icon name="file-text" size={14} /></span>
                    <span class="note-title">{n.title}</span>
                    {#if noteBacklinks.length > 0}
                      <span class="backlink-pill" use:tip={`${noteBacklinks.length} backlink(s)`}><Icon name="link" size={11} /> {noteBacklinks.length}</span>
                    {/if}
                  </div>
                  <div class="note-actions">
                    <button
                      type="button"
                      class="btn-icon"
                      use:tip={"Edit note"}
                      onclick={(e) => {
                        e.stopPropagation();
                        editing = { path: n.path, base: n.content };
                        draft = n.content;
                        editorPreview = false;
                      }}
                    >
                      <Icon name="pencil" size={13} /> Edit
                    </button>
                    <button
                      type="button"
                      class="btn-icon btn-danger-soft"
                      use:tip={"Delete note"}
                      onclick={(e) => {
                        e.stopPropagation();
                        deleteRecord(n.path, n.title);
                      }}
                    >
                      <Icon name="trash" size={13} /> Delete
                    </button>
                  </div>
                </summary>
                <div class="note-content-wrap">
                  {@render editable(n)}
                  {#if noteBacklinks.length > 0}
                    <div class="backlinks-section">
                      <span class="backlinks-label">Referenced by ({noteBacklinks.length}):</span>
                      <div class="backlinks-list">
                        {#each noteBacklinks as bl (bl.path)}
                          {@const proj = recordProject(bl.path)}
                          <a
                            href={link(`/projects?project=${encodeURIComponent(proj)}&open=${encodeURIComponent(bl.path)}`)}
                            class="backlink-chip"
                          >
                            <span class="chip-kind">{bl.kind}</span>
                            <span class="chip-title">{bl.title}</span>
                          </a>
                        {/each}
                      </div>
                    </div>
                  {/if}
                </div>
              </details>
            {/each}
          </div>
        {/if}

        {#if canAdd}
          <form class="add-note-form" onsubmit={addNote}>
            <input aria-label="New note" placeholder="New note title…" bind:value={noteTitle} maxlength="200" required />
            <button type="submit" class="btn" disabled={busy || !noteTitle.trim()}>+ Add note</button>
          </form>
        {/if}
      {/if}
    {/if}
  </section>
</div>

{#snippet editable(record: ProjectRecord)}
  <div class="note-top-bar">
    {#if editing && editing.path === record.path}
      <div class="edit-toolbar-top">
        <div class="edit-mode-tabs">
          <button
            type="button"
            class="tab-btn"
            class:active={!editorPreview}
            onclick={() => (editorPreview = false)}
          >
            <Icon name="pencil" size={13} /> Markdown
          </button>
          <button
            type="button"
            class="tab-btn"
            class:active={editorPreview}
            onclick={() => (editorPreview = true)}
          >
            <Icon name="eye" size={13} /> Preview
          </button>
        </div>
        <div class="edit-actions-top">
          <details class="genui-dropdown">
            <summary class="btn btn-sm btn-soft"><Icon name="sparkles" size={13} /> + GenUI ▾</summary>
            <div class="genui-menu">
              <button type="button" class="genui-menu-item" onclick={() => insertGenUI("metrics")}><Icon name="activity" size={13} /> Metric Grid</button>
              <button type="button" class="genui-menu-item" onclick={() => insertGenUI("matrix")}><Icon name="layout" size={13} /> Decision Matrix</button>
              <button type="button" class="genui-menu-item" onclick={() => insertGenUI("simulator")}><Icon name="cpu" size={13} /> Simulator</button>
              <button type="button" class="genui-menu-item" onclick={() => insertGenUI("timeline")}><Icon name="clock" size={13} /> Timeline</button>
            </div>
          </details>
          <button
            type="button"
            class="btn btn-sm btn-primary"
            disabled={busy || draft === editing.base}
            onclick={saveEdit}
          >
            <Icon name="check" size={13} /> Save (⌘S)
          </button>
          <button
            type="button"
            class="btn btn-sm btn-soft"
            disabled={busy}
            onclick={() => (editing = null)}
          >
            <Icon name="close" size={13} /> Cancel
          </button>
        </div>
      </div>
    {:else}
      <div class="view-toolbar-top">
        <div class="record-meta-top">
          <span class="kind-tag">{record.kind}</span>
          {#if record.due}<span class="meta-tag">Due: {record.due}</span>{/if}
          {#if record.priority}<span class="meta-tag">P{record.priority}</span>{/if}
        </div>
        <div class="view-actions-top">
          <button
            type="button"
            class="btn-icon"
            use:tip={"Edit note"}
            onclick={() => {
              editing = { path: record.path, base: record.content };
              draft = record.content;
              editorPreview = false;
            }}
          >
            <Icon name="pencil" size={13} /> Edit
          </button>
          {#if record.kind !== "project"}
            <button
              type="button"
              class="btn-icon btn-danger-soft"
              use:tip={"Delete note"}
              onclick={() => deleteRecord(record.path, record.title)}
            >
              <Icon name="trash" size={13} /> Delete
            </button>
          {/if}
        </div>
      </div>
    {/if}
  </div>

  {#if editing && editing.path === record.path}
    {#if editorPreview}
      <div class="editor-live-preview">
        {#each numbered(segments(draft.replace(/^---\n[\s\S]*?\n---\n/, ""), record.path, files)) as { part, first }, i (i)}
          {#if "markdown" in part}
            <MarkdownDocument
              content={body(part.markdown, record.path)}
              compact
              resolveLink={(href) => href}
            />
          {:else if "file" in part}
            {#if part.image}
              <img class="attachment" src={fileUrl(part.file)} alt={part.file.split("/").pop()} loading="lazy" />
            {:else}
              <p class="vault-base"><a href={fileUrl(part.file)} target="_blank" rel="noreferrer">{part.file.split("/").pop()}</a></p>
            {/if}
          {:else if "figure" in part}
            <div class="figure-embed">
              {#if figures[part.figure]}
                {@const Fig = figures[part.figure].component}
                <Fig title={part.label ?? figures[part.figure].title} />
              {:else}
                <p class="error">Unregistered figure: <code>{part.figure}</code></p>
              {/if}
            </div>
          {:else if "genui" in part}
            <GenUIRenderer spec={part.genui} />
          {/if}
        {/each}
      </div>
    {:else}
      <textarea
        aria-label={`Edit ${record.title}`}
        bind:value={draft}
        rows="20"
        spellcheck="false"
        placeholder="Write in markdown…"
        onkeydown={(e) => {
          if ((e.metaKey || e.ctrlKey) && e.key === "s") {
            e.preventDefault();
            void saveEdit();
          } else if (e.key === "Escape") {
            editing = null;
          }
        }}
      ></textarea>
    {/if}
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
      {:else if "figure" in part}
        <div class="figure-embed">
          {#if figures[part.figure]}
            {@const Fig = figures[part.figure].component}
            <Fig title={part.label ?? figures[part.figure].title} />
          {:else}
            <p class="error">Unregistered figure: <code>{part.figure}</code></p>
          {/if}
        </div>
      {:else if "genui" in part}
        <GenUIRenderer spec={part.genui} />
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

  .board {
    display: grid;
    grid-template-columns: minmax(13rem, 17rem) 1fr;
    gap: 1.5rem;
    align-items: start;
    position: relative;
  }

  @media (max-width: 720px) {
    .board {
      grid-template-columns: 1fr;
    }
  }

  /* Nav & Notion-style Project sidebar */
  .nav-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0 0.2rem 0.5rem;
    margin-bottom: 0.4rem;
    border-bottom: 1px solid var(--rule);
  }

  .nav-title {
    font-weight: 700;
    font-size: var(--text-sm);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-secondary);
  }

  .btn-new-project {
    background: transparent;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    padding: 0.15rem 0.45rem;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--primary);
    cursor: pointer;
    transition: background var(--motion-fast) ease, border-color var(--motion-fast) ease;
  }

  .btn-new-project:hover {
    background: var(--primary-soft);
    border-color: var(--primary);
  }

  .new-project-form {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    padding: 0.5rem;
    margin-bottom: 0.6rem;
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
  }

  .new-project-form input {
    font-size: var(--text-xs);
    padding: 0.3rem 0.4rem;
  }

  .form-row-btns {
    display: flex;
    gap: 0.35rem;
    justify-content: flex-end;
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
    transition: background var(--motion-fast) ease;
  }

  nav a:hover {
    background: var(--surface);
  }

  .name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .counts {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .notes-count {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    font-size: var(--text-2xs);
    color: var(--text-secondary);
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

  /* Project Toolbar */
  .project-toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 1rem;
    padding-bottom: 0.6rem;
    border-bottom: 1px solid var(--rule);
    margin-bottom: 1rem;
  }

  .project-heading-cluster {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .project-heading-cluster h2 {
    margin: 0;
    font-size: var(--text-lg);
  }

  .archived-pill {
    font-size: var(--text-xs);
    padding: 0.15rem 0.45rem;
    border-radius: 4px;
    background: var(--rule);
    color: var(--text-secondary);
  }

  .project-actions {
    display: flex;
    gap: 0.4rem;
  }

  h2, h3 {
    margin: 1.25rem 0 0.5rem;
  }

  h3 {
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
    align-items: center;
    gap: 1rem;
    padding: 0.4rem 0.2rem;
    border-bottom: 1px solid var(--rule);
  }

  .task-right-meta {
    display: flex;
    align-items: center;
    gap: 0.6rem;
  }

  .btn-icon-subtle {
    background: transparent;
    border: none;
    color: var(--text-secondary);
    cursor: pointer;
    font-size: 0.8rem;
    padding: 0.1rem 0.3rem;
    border-radius: 3px;
  }

  .btn-icon-subtle:hover {
    color: var(--danger);
    background: var(--danger-soft, #fee2e2);
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

  /* Form styling */
  .add-task-form,
  .add-note-form {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin-top: 0.75rem;
  }

  .add-task-form input:first-child,
  .add-note-form input:first-child {
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

  /* Notion-Style Note Cards */
  .notes-stack {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .note {
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    background: var(--card-bg, #ffffff);
    overflow: hidden;
    transition: border-color var(--motion-fast) ease;
  }

  .note:hover {
    border-color: var(--primary);
  }

  .note summary {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 0.6rem 0.8rem;
    background: var(--surface);
    font-weight: 600;
    color: var(--text-primary);
  }

  .note-summary-main {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    min-width: 0;
  }

  .note-icon {
    font-size: 1rem;
  }

  .note-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .backlink-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.2rem;
    font-size: var(--text-2xs);
    padding: 0.1rem 0.35rem;
    background: var(--rule);
    border-radius: 4px;
    font-weight: 500;
    color: var(--text-secondary);
  }

  .note-actions {
    display: flex;
    gap: 0.4rem;
  }

  .btn-icon {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    background: transparent;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    padding: 0.15rem 0.45rem;
    font-size: var(--text-xs);
    color: var(--text-secondary);
    cursor: pointer;
    transition: background var(--motion-fast) ease, color var(--motion-fast) ease, border-color var(--motion-fast) ease;
  }

  .btn-icon:hover {
    background: var(--card-bg);
    color: var(--text-primary);
  }

  .btn-danger-soft {
    color: var(--danger, #dc2626);
  }

  .btn-danger-soft:hover {
    background: var(--danger-soft, #fee2e2);
    border-color: var(--danger, #dc2626);
  }

  .note-content-wrap {
    padding: 0.8rem;
    border-top: 1px solid var(--rule);
  }

  /* Backlinks ("Referenced by") section */
  .backlinks-section {
    margin-top: 1rem;
    padding-top: 0.6rem;
    border-top: 1px dashed var(--rule);
  }

  .backlinks-label {
    display: block;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-secondary);
    margin-bottom: 0.4rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }

  .backlinks-list {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }

  .backlink-chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: var(--surface);
    border: 1px solid var(--rule);
    border-radius: 4px;
    padding: 0.2rem 0.5rem;
    font-size: var(--text-xs);
    color: var(--text-primary);
    text-decoration: none;
    transition: background var(--motion-fast) ease, border-color var(--motion-fast) ease, color var(--motion-fast) ease;
  }

  .backlink-chip:hover {
    background: var(--primary-soft);
    border-color: var(--primary);
    color: var(--primary);
  }

  .chip-kind {
    font-size: 0.65rem;
    text-transform: uppercase;
    font-weight: 700;
    color: var(--text-secondary);
  }

  .chip-title {
    font-weight: 500;
  }

  /* Opened Task Card */
  .opened {
    background: var(--surface);
    border: 1px solid var(--primary);
    border-radius: var(--radius-sm);
    padding: 0.8rem;
    margin: 0.8rem 0;
  }

  .opened-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 0.6rem;
    border-bottom: 1px solid var(--rule);
    padding-bottom: 0.4rem;
  }

  .opened-title-wrap {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .opened-title-wrap h3 {
    margin: 0;
  }

  .opened-badge {
    background: var(--primary-soft);
    color: var(--primary);
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .opened-actions {
    display: flex;
    gap: 0.4rem;
  }

  .error {
    color: var(--danger);
  }

  /* Floating Hover Preview Card */
  .hover-preview-card {
    position: fixed;
    z-index: 1000;
    width: 23rem;
    max-width: calc(100vw - 2rem);
    max-height: 22rem;
    display: flex;
    flex-direction: column;
    background: var(--card-bg, #ffffff);
    border: 1px solid var(--primary, #2563eb);
    border-radius: var(--radius-sm, 6px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.16);
    padding: 0.75rem 0.9rem;
    pointer-events: auto;
    animation: fadeIn var(--motion-fast) ease-out;
  }

  @keyframes fadeIn {
    from {
      opacity: 0;
      transform: translateY(3px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .hover-preview-header {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin-bottom: 0.35rem;
  }

  .hover-preview-badge {
    text-transform: uppercase;
    font-size: 0.65rem;
    font-weight: 700;
    background: var(--primary-soft, #eff6ff);
    color: var(--primary, #2563eb);
    padding: 0.1rem 0.35rem;
    border-radius: 3px;
  }

  .hover-preview-proj {
    font-size: 0.68rem;
    color: var(--text-secondary, #64748b);
    background: var(--surface, #f8fafc);
    padding: 0.1rem 0.35rem;
    border-radius: 3px;
    border: 1px solid var(--rule, #e2e8f0);
  }

  .hover-preview-mat {
    font-size: 0.68rem;
    color: #047857;
    background: #ecfdf5;
    padding: 0.1rem 0.35rem;
    border-radius: 3px;
    border: 1px solid #a7f3d0;
  }

  .hover-preview-status {
    font-size: 0.68rem;
    color: #b45309;
    background: #fffbeb;
    padding: 0.1rem 0.35rem;
    border-radius: 3px;
  }

  .hover-preview-prio,
  .hover-preview-due {
    font-size: 0.7rem;
    color: var(--text-secondary, #64748b);
  }

  .hover-preview-title {
    font-weight: 600;
    font-size: var(--text-sm, 0.85rem);
    color: var(--text-primary, #0f172a);
    margin-bottom: 0.25rem;
    line-height: 1.3;
  }

  .hover-preview-title a {
    color: inherit;
    text-decoration: none;
  }

  .hover-preview-title a:hover {
    color: var(--primary, #2563eb);
    text-decoration: underline;
  }

  .hover-preview-scroll-area {
    flex: 1 1 auto;
    max-height: 13rem;
    overflow-y: auto;
    margin: 0.35rem 0;
    padding-right: 0.4rem;
    scrollbar-width: thin;
    scrollbar-color: var(--rule, #cbd5e1) transparent;
  }

  .hover-preview-excerpt {
    font-size: var(--text-xs, 0.75rem);
    color: var(--text-secondary, #475569);
    line-height: 1.45;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .hover-preview-footer {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-top: 0.4rem;
    padding-top: 0.4rem;
    border-top: 1px solid var(--rule, #e2e8f0);
    font-size: var(--text-2xs);
  }

  .hover-preview-bl {
    color: var(--text-secondary, #64748b);
  }

  .hover-preview-open-link {
    color: var(--primary, #2563eb);
    text-decoration: none;
    font-weight: 600;
  }

  .hover-preview-open-link:hover {
    text-decoration: underline;
  }

  /* Notes View Switcher & Table */
  .notes-header-bar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin: 1.25rem 0 0.5rem;
  }

  .notes-header-bar h3 {
    margin: 0;
  }

  .notes-view-switcher {
    display: flex;
    gap: 0.3rem;
    background: var(--surface);
    padding: 0.2rem;
    border-radius: var(--radius-sm);
    border: 1px solid var(--rule);
  }

  .view-switch-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: transparent;
    border: none;
    font-size: var(--text-xs);
    padding: 0.2rem 0.5rem;
    border-radius: 4px;
    color: var(--text-secondary);
    cursor: pointer;
    transition: color var(--motion-fast) ease, background var(--motion-fast) ease;
  }

  .view-switch-btn:hover {
    color: var(--text-primary);
  }

  .view-switch-btn.active {
    background: var(--card-bg, #ffffff);
    color: var(--primary);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.05);
  }

  .notes-table-wrap {
    margin-top: 0.5rem;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    overflow-x: auto;
    background: var(--card-bg, #ffffff);
    padding: 0.5rem;
  }

  /* Note Top Edit Toolbar */
  .note-top-bar {
    margin-bottom: 0.8rem;
    padding-bottom: 0.5rem;
    border-bottom: 1px solid var(--rule);
  }

  .edit-toolbar-top,
  .view-toolbar-top {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 0.6rem;
  }

  .edit-mode-tabs {
    display: flex;
    gap: 0.3rem;
  }

  .tab-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: transparent;
    border: 1px solid var(--rule);
    border-radius: var(--radius-sm);
    padding: 0.2rem 0.55rem;
    font-size: var(--text-xs);
    cursor: pointer;
    color: var(--text-secondary);
    transition: color var(--motion-fast) ease, background var(--motion-fast) ease, border-color var(--motion-fast) ease;
  }

  .tab-btn:hover {
    color: var(--text-primary);
  }

  .tab-btn.active {
    background: var(--primary-soft);
    border-color: var(--primary);
    color: var(--primary);
    font-weight: 600;
  }

  .edit-actions-top,
  .view-actions-top {
    display: flex;
    gap: 0.4rem;
  }

  .btn-primary {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    background: var(--primary);
    color: #ffffff;
    border: 1px solid var(--primary);
  }

  .btn-primary:hover {
    opacity: 0.92;
  }

  .record-meta-top {
    display: flex;
    align-items: center;
    gap: 0.4rem;
  }

  .kind-tag {
    text-transform: uppercase;
    font-size: var(--text-2xs);
    font-weight: 700;
    background: var(--surface);
    border: 1px solid var(--rule);
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
    color: var(--text-secondary);
  }

  .meta-tag {
    font-size: var(--text-xs);
    color: var(--text-secondary);
    background: var(--surface);
    padding: 0.1rem 0.4rem;
    border-radius: 3px;
  }

  .editor-live-preview {
    border: 1px dashed var(--rule);
    border-radius: var(--radius-sm);
    padding: 0.8rem;
    background: var(--surface);
    margin-bottom: 0.8rem;
  }

  .genui-dropdown {
    position: relative;
    display: inline-block;
  }

  .genui-dropdown summary {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    list-style: none;
    cursor: pointer;
  }

  .genui-dropdown summary::-webkit-details-marker {
    display: none;
  }

  .genui-menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 20;
    display: flex;
    flex-direction: column;
    min-width: 11.5rem;
    background: var(--surface, #ffffff);
    border: 1px solid var(--rule, #e2e8f0);
    border-radius: var(--radius-sm, 6px);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.1);
    padding: 0.3rem;
  }

  .genui-menu-item {
    all: unset;
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.35rem 0.6rem;
    font-size: var(--text-xs, 0.75rem);
    border-radius: 4px;
    color: var(--text-primary, #0f172a);
    text-align: left;
    transition: background-color var(--motion-fast) ease;
  }

  .genui-menu-item:hover {
    background: var(--surface, #f1f5f9);
    color: var(--primary, #2563eb);
  }
</style>
