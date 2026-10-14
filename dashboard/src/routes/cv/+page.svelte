<script lang="ts">
  // The CV: build a profile, track applications, and keep LinkedIn in step with the master.
  // Every button is one `capabilities/cv/cv` verb run by sjel-status, so a refusal is the
  // launcher's own words and the page re-reads state after every write.
  import { onMount } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { axonStatus, type CvView, type CvWrite } from "$lib/api";
  import { inTauri } from "$lib/mac-bridge";
  import Chip from "$lib/ui/Chip.svelte";

  let view = $state<CvView | null>(null);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let writeError = $state<string | null>(null);
  /** stdout of the last verb: a built path, a LinkedIn diff or a paste sheet. */
  let output = $state<string | null>(null);
  let preview = $state<string | null>(null);

  let profile = $state("default");
  let lang = $state<"en" | "de">("en");
  let draft = $state({ company: "", role: "", url: "", profile: "default", lang: "en" as "en" | "de" });

  const web = !inTauri();
  const slugFor = (company: string, role: string) =>
    `${company}-${role}`.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 48);

  async function load(): Promise<void> {
    try {
      view = await axonStatus.cv();
      error = null;
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function run(write: CvWrite, show = true): Promise<void> {
    busy = true;
    writeError = null;
    try {
      const result = await axonStatus.writeCv(write);
      if (show) output = [result.detail, result.warnings].filter(Boolean).join("\n\n");
    } catch (e) {
      writeError = e instanceof Error ? e.message : String(e);
    } finally {
      busy = false;
      await load();
    }
  }

  async function addApplication(): Promise<void> {
    await run({ action: "app-new", slug: slugFor(draft.company, draft.role), ...draft });
    if (!writeError) draft = { ...draft, company: "", role: "", url: "" };
  }

  const last = (log: { date: string }[]) => log.at(-1)?.date ?? "";
  const ago = (mtime: number) => new Date(mtime * 1000).toISOString().slice(0, 10);

  onMount(() => void load());
</script>

<PageHeader badge="CV" title="CV, applications and LinkedIn" />

{#if error}
  <div class="card pad">
    <p class="err"><Icon name="alert" /> Could not read the CV state</p>
    <p class="hint">sjel-status runs <code>capabilities/cv/cv status --json</code>.</p>
    <p class="hint">{error}</p>
  </div>
{:else if !view}
  <p class="hint"><Icon name="loader" /> Reading the master…</p>
{:else}
  {#if writeError}<p class="err"><Icon name="alert" /> {writeError}</p>{/if}

  <section class="card pad">
    <h3>Build</h3>
    <p class="hint">Master: <code>{view.master}</code>, edited {ago(view.master_mtime)}</p>
    <div class="row">
      <select aria-label="Profile" bind:value={profile}>{#each view.profiles as p}<option>{p}</option>{/each}</select>
      <select aria-label="Language" bind:value={lang}><option>en</option><option>de</option></select>
      <button class="btn btn-soft" disabled={busy} onclick={() => run({ action: "build", profile, lang })}>Build</button>
      <button class="btn btn-outline" disabled={busy} onclick={() => run({ action: "build-all" })}>Build all</button>
    </div>
  </section>

  <section class="card pad">
    <h3>Applications</h3>
    {#if view.applications.length === 0}<p class="hint">None yet.</p>{/if}
    <table>
      <tbody>
        {#each view.applications as app (app.slug)}
          <tr>
            <td><strong>{app.company}</strong><br /><span class="hint">{app.role}</span></td>
            <td class="hint">{app.profile} · {app.lang}</td>
            <td>
              <select
                aria-label="Application status"
                value={app.status}
                disabled={busy}
                onchange={(e) => run({ action: "app-status", slug: app.slug, status: e.currentTarget.value }, false)}
              >
                {#each view.statuses as s}<option>{s}</option>{/each}
              </select>
            </td>
            <td class="hint">{last(app.log)}</td>
            <td>
              {#if app.url}<a href={app.url} target="_blank" rel="noreferrer"><Icon name="external" size={12} /></a>{/if}
              <button class="btn btn-soft" disabled={busy} onclick={() => run({ action: "app-build", slug: app.slug })}>Build</button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    <div class="row">
      <input aria-label="Company" placeholder="Company" bind:value={draft.company} />
      <input aria-label="Role" placeholder="Role" bind:value={draft.role} />
      <input aria-label="Posting URL" placeholder="Posting URL" bind:value={draft.url} />
      <select aria-label="Draft profile" bind:value={draft.profile}>{#each view.profiles as p}<option>{p}</option>{/each}</select>
      <select aria-label="Draft language" bind:value={draft.lang}><option>en</option><option>de</option></select>
      <button class="btn btn-soft" disabled={busy || !draft.company || !draft.role} onclick={addApplication}>
        <Icon name="plus" size={12} /> Add
      </button>
    </div>
  </section>

  <section class="card pad">
    <h3>LinkedIn</h3>
    <p class="hint">Last pulled: {view.linkedin_pulled ?? "never"}. LinkedIn has no write API; paste from the sheet.</p>
    <div class="row">
      <button class="btn btn-outline" disabled={busy} onclick={() => run({ action: "linkedin-pull" })}>Pull</button>
      <button class="btn btn-outline" disabled={busy || !view.linkedin_pulled} onclick={() => run({ action: "linkedin-diff" })}>Diff</button>
      <button class="btn btn-outline" disabled={busy} onclick={() => run({ action: "linkedin-sheet", lang })}>Paste sheet ({lang})</button>
    </div>
  </section>

  {#if output}
    <section class="card pad">
      <div class="row">
        <h3>Output</h3>
        <button class="btn btn-outline" onclick={() => navigator.clipboard.writeText(output ?? "")}>Copy</button>
      </div>
      <pre>{output}</pre>
    </section>
  {/if}

  <section class="card pad">
    <h3>PDFs</h3>
    <ul>
      {#each view.pdfs as pdf (pdf.name)}
        <li>
          {#if web}
            <button class="link" onclick={() => (preview = pdf.name)}>{pdf.name}</button>
          {:else}
            <code>{pdf.path}</code>
          {/if}
          {#if pdf.stale}<Chip label="older than master" tone="warning" />{/if}
        </li>
      {/each}
    </ul>
    {#if preview && web}
      <iframe title={preview} src={axonStatus.cvPdfUrl(preview)}></iframe>
    {/if}
  </section>
{/if}

<style>
  .pad { padding: 0.8rem 0.9rem; margin: 0 0 0.8rem; }
  h3 { font-size: 0.9rem; margin: 0 0 0.4rem; }
  .hint { font-size: var(--text-xs); color: var(--text-secondary); margin: 0 0 0.3rem; }
  .err { display: flex; gap: 0.45rem; font-size: 0.85rem; color: var(--warning-ink); }
  .row { display: flex; flex-wrap: wrap; align-items: center; gap: 0.35rem; margin: 0.4rem 0 0; }
  input, select {
    font-size: var(--text-xs);
    padding: 0.15rem 0.35rem;
    background: var(--input-bg);
    color: var(--text-primary);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
  }
  table { width: 100%; border-collapse: collapse; font-size: 0.8rem; }
  td { padding: 0.3rem 0.4rem 0.3rem 0; vertical-align: top; }
  ul { list-style: none; padding: 0; margin: 0; font-size: 0.78rem; }
  li { display: flex; align-items: center; gap: 0.4rem; padding: 0.12rem 0; }
  .link { background: none; border: 0; padding: 0; color: var(--text-primary); text-decoration: underline; cursor: pointer; }
  pre { font-size: 0.72rem; white-space: pre-wrap; margin: 0.4rem 0 0; max-height: 28rem; overflow: auto; }
  iframe { width: 100%; height: 80vh; border: 1px solid var(--card-border); border-radius: var(--radius-sm); margin: 0.5rem 0 0; }
</style>
