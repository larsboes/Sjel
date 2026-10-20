<script lang="ts">
  import { page } from "$app/state";
  import { replaceState } from "$app/navigation";
  import { documents, type DocumentDetail, type DocumentSummary } from "$lib/api/documents";
  import { acquireBridgedUrl, releaseBridgedUrl } from "$lib/bridged-url";
  import PageHeader from "$lib/PageHeader.svelte";
  import StateLine from "$lib/StateLine.svelte";
  import Icon from "$lib/Icon.svelte";
  import Collection from "$lib/ui/Collection.svelte";
  import Chip from "$lib/ui/Chip.svelte";
  import Freshness from "$lib/ui/Freshness.svelte";
  import { poll } from "$lib/ui/poll.svelte";
  import type { Field } from "$lib/ui/collection";
  import { tip } from "$lib/tip";

  const library = poll({ intervalMs: 30_000, fetch: documents.list });
  $effect(() => library.start());

  const rows = $derived(library.data ?? []);
  const inboxCount = $derived(rows.filter((d) => d.status === "inbox").length);
  const selectedId = $derived(page.url.searchParams.get("document"));
  let detail = $state.raw<DocumentDetail | null>(null);
  let loadingDetail = $state(false);
  let detailError = $state<string | null>(null);
  let actionError = $state<string | null>(null);
  let notice = $state<string | null>(null);
  let busy = $state(false);
  let category = $state("");
  let project = $state("");
  let mode = $state("original");
  let propertiesOpen = $state(false);
  let fileInput = $state<HTMLInputElement>();
  let originalUrl = $state<string | null>(null);
  let originalError = $state<string | null>(null);

  const fields: Field<DocumentSummary>[] = [
    { id: "filename", label: "Document", value: (d) => d.filename, cell: filenameCell },
    { id: "status", label: "Status", kind: "select", value: (d) => d.status,
      display: (s) => ({ inbox: "Inbox", filed: "Filed", archived: "Archived" })[s] ?? s,
      tone: (s) => s === "inbox" ? "warning" : s === "archived" ? "muted" : "success" },
    { id: "category", label: "Category", kind: "select", value: (d) => d.category ?? "Unfiled" },
    { id: "project", label: "Project", value: (d) => d.project, column: false },
    { id: "title", label: "Title", value: (d) => d.extracted_title, column: false },
    { id: "imported", label: "Added", kind: "date", value: (d) => d.imported_at, column: false },
  ];

  const message = (error: unknown) => error instanceof Error ? error.message : String(error);

  function select(id: string | null) {
    const url = new URL(page.url);
    if (id) url.searchParams.set("document", id);
    else url.searchParams.delete("document");
    replaceState(url, page.state);
  }

  $effect(() => {
    const id = selectedId;
    detail = null;
    detailError = null;
    propertiesOpen = false;
    mode = "original";
    if (!id) { loadingDetail = false; return; }
    let active = true;
    loadingDetail = true;
    void documents.detail(id).then(
      (result) => { if (active) detail = result; },
      (error) => { if (active) detailError = message(error); },
    ).finally(() => { if (active) loadingDetail = false; });
    return () => { active = false; };
  });

  $effect(() => {
    category = detail?.document.category ?? "";
    project = detail?.document.project ?? "";
  });

  $effect(() => {
    const document = detail?.document;
    originalUrl = null;
    originalError = null;
    if (!document || document.media_type !== "application/pdf" || mode !== "original") return;
    const path = documents.originalUrl(document.id);
    let active = true;
    void acquireBridgedUrl(path).then(
      (url) => { if (active) originalUrl = url; },
      (error) => { if (active) originalError = message(error); },
    );
    return () => { active = false; releaseBridgedUrl(path); };
  });

  async function upload(event: Event) {
    const input = event.currentTarget as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    busy = true; actionError = null; notice = null;
    try {
      if (file.size === 0 || file.size > 128 * 1024 * 1024) throw new Error("Choose a non-empty document up to 128 MiB.");
      const result = await documents.upload(file);
      notice = result.created ? `Added ${result.document.filename} to the inbox.` : "This document is already in your library.";
      await library.refresh();
      select(result.document.id);
    } catch (error) { actionError = message(error); }
    finally { busy = false; input.value = ""; }
  }

  async function saveCategory(event: SubmitEvent) {
    event.preventDefault();
    const id = detail?.document.id;
    if (!id || !category.trim()) return;
    busy = true; actionError = null; notice = null;
    const release = library.hold();
    try {
      const updated = await documents.review(id, category.trim(), project.trim() || null);
      if (detail?.document.id === id) detail = { ...detail, document: updated };
      await library.refresh();
      notice = `Filed as ${updated.category}.`;
    } catch (error) { actionError = message(error); }
    finally { busy = false; release(); }
  }

  async function archive() {
    const id = detail?.document.id;
    if (!id) return;
    busy = true; actionError = null; notice = null;
    const release = library.hold();
    try {
      const updated = await documents.archive(id);
      if (detail?.document.id === id) detail = { ...detail, document: updated };
      await library.refresh();
      notice = "Archived. The original is retained; file it again to restore it.";
    } catch (error) { actionError = message(error); }
    finally { busy = false; release(); }
  }

  function formatSize(bytes: number) {
    return bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  }
</script>

{#snippet actions()}
  <button type="button" class="btn btn-primary" disabled={busy} onclick={() => fileInput?.click()}>
    <Icon name="plus" size={14} /> Add document
  </button>
  <input bind:this={fileInput} type="file" aria-label="Upload a document" onchange={upload}
    accept=".pdf,.txt,.md,.doc,.docx,.xls,.xlsx,.ppt,.pptx,.odt,.ods,.odp,.rtf,.csv,.epub" hidden />
{/snippet}

{#snippet context()}
  <span>{rows.length} documents · {inboxCount} in inbox</span>
  <Freshness of={library} />
{/snippet}

{#snippet filenameCell(document: DocumentSummary)}
  <span class="document-name"><Icon name="file-text" size={14} />{document.filename}</span>
{/snippet}

<PageHeader badge="Library" title="Documents" desc="Your originals, filing, and document contents stay in Sjel." {actions} {context} />

{#if actionError}<StateLine state="error" message={actionError} />{/if}
{#if notice}<p class="notice" role="status">{notice}</p>{/if}
{#if library.error}<StateLine state="error" message={library.error} onRetry={() => library.refresh()} />{/if}
{#if !library.data && !library.error}<StateLine state="loading" message="Loading your documents…" />{/if}

<div class={['library', selectedId && 'with-reader']}>
  <section class={['document-list', selectedId && 'has-selection']} aria-label="Document library">
    {#if library.data}
      <Collection id="documents" {rows} {fields} key={(d) => d.id} title={(d) => d.filename}
        selected={selectedId} onOpen={(d) => select(d.id)} inactive={(d) => d.status === 'archived'}
        empty="Add a document to begin, or adjust your filters." />
      <p class="list-note">Search filenames and titles. Filter by status or category.</p>
    {/if}
  </section>

  {#if selectedId}
    <section class="reader" aria-label="Document viewer">
      <div class="reader-toolbar">
        <button class="btn" type="button" onclick={() => select(null)} use:tip={"Close document"}><Icon name="close" size={14} /></button>
        <h2>{detail?.document.filename ?? "Document"}</h2>
        {#if detail}
          <button class="btn" type="button" aria-expanded={propertiesOpen} onclick={() => propertiesOpen = !propertiesOpen}>Filing</button>
          <a class="btn" href={documents.originalUrl(detail.document.id, true)} download>Download</a>
        {/if}
      </div>
      {#if loadingDetail}<StateLine state="loading" message="Opening document…" />{/if}
      {#if detailError}<StateLine state="error" message={detailError} />{/if}
      {#if detail}
        {@const document = detail.document}
        <div class="reader-meta">
          <Chip label={document.status} tone={document.status === "inbox" ? "warning" : "neutral"} />
          <span>{document.category ?? "Unfiled"}</span>
          <span class="mono">{formatSize(document.byte_len)}</span>
          <span>Local · {document.data_class}</span>
        </div>
        {#if propertiesOpen || document.status === "inbox"}
          <form class="filing" onsubmit={saveCategory}>
            <label>Category <input class="input" name="category" bind:value={category} list="document-categories" required maxlength="120" disabled={busy} /></label>
            <datalist id="document-categories">
              {#each [...new Set(["Finance", "Career", "Reference", "Events", ...rows.flatMap((d) => d.category ? [d.category] : [])])] as name (name)}<option value={name}></option>{/each}
            </datalist>
            <label>Project <input class="input" name="project" bind:value={project} placeholder="Optional" maxlength="120" disabled={busy} /></label>
            <button class="btn btn-primary" type="submit" disabled={busy || !category.trim()}>File document</button>
            {#if document.suggested_category && !category}
              <button type="button" class="btn" onclick={() => category = document.suggested_category ?? ""}>Use suggestion: {document.suggested_category}</button>
            {/if}
            {#if document.status !== "archived" && propertiesOpen}
              <button type="button" class="btn archive" disabled={busy} onclick={archive}>Archive · retain original</button>
            {/if}
          </form>
        {/if}
        {#if document.media_type === "application/pdf" && detail.text}
          <div class="reader-tabs" role="group" aria-label="Document view">
            <button class="btn" type="button" aria-pressed={mode === "original"} onclick={() => mode = "original"}>Original PDF</button>
            <button class="btn" type="button" aria-pressed={mode === "text"} onclick={() => mode = "text"}>Extracted text</button>
          </div>
        {/if}
        {#if document.media_type === "application/pdf" && mode === "original"}
          {#if originalError}<StateLine state="error" message={originalError} />
          {:else if originalUrl}<iframe class="pdf" src={originalUrl} title={document.filename}></iframe>
          {:else}<StateLine state="loading" message="Loading the original PDF…" />{/if}
        {:else if detail.text}
          <pre class="document-text">{detail.text}</pre>
        {:else}
          <div class="no-preview"><Icon name="file-text" size={24} /><p>Preview is not available for this format.</p><a class="btn" href={documents.originalUrl(document.id, true)} download>Download original</a></div>
        {/if}
      {/if}
    </section>
  {/if}
</div>

<style>
  .library { display: grid; gap: var(--space-5); align-items: start; }
  .with-reader { grid-template-columns: minmax(20rem, 1fr) minmax(0, 1.6fr); }
  .document-list { min-width: 0; }
  .document-name { display: inline-flex; align-items: center; gap: var(--space-2); }
  .list-note { font-size: var(--text-xs); color: var(--text-tertiary); margin-top: var(--space-3); }
  .reader { min-width: 0; border: 1px solid var(--card-border); border-radius: var(--radius-md); overflow: hidden; background: var(--card-bg); }
  .reader-toolbar { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-3); border-bottom: 1px solid var(--rule); }
  h2 { flex: 1; min-width: 0; font-size: var(--text-sm); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .reader-meta { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-3); padding: var(--space-3); font-size: var(--text-xs); color: var(--text-secondary); }
  .filing { display: flex; flex-wrap: wrap; align-items: end; gap: var(--space-3); padding: var(--space-3); border-bottom: 1px solid var(--rule); }
  label { display: grid; gap: var(--space-1); font-size: var(--text-xs); color: var(--text-secondary); flex: 1 1 9rem; }
  .input { width: 100%; min-width: 0; }
  .archive { color: var(--text-tertiary); }
  .reader-tabs { display: flex; gap: var(--space-2); padding: var(--space-3); }
  .reader-tabs [aria-pressed="true"] { background: var(--primary-soft); color: var(--primary); }
  .pdf { display: block; width: 100%; height: max(32rem, calc(100dvh - 20rem)); border: 0; }
  .document-text { white-space: pre-wrap; overflow-wrap: anywhere; font-family: var(--font-sans); font-size: var(--text-sm); line-height: var(--leading-relaxed); padding: var(--space-5); max-height: calc(100dvh - 18rem); overflow-y: auto; }
  .no-preview { display: grid; justify-items: center; gap: var(--space-4); padding: var(--space-8); color: var(--text-secondary); font-size: var(--text-sm); }
  .notice { font-size: var(--text-sm); color: var(--text-secondary); padding-block: var(--space-3); }
  @media (width < 64rem) { .with-reader { grid-template-columns: 1fr; } .has-selection { display: none; } }
</style>
