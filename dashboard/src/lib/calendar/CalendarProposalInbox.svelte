<script lang="ts">
  import { calendar, type CalendarEntry } from "$lib/api";
  import { KINDS } from "./types";

  let {
    refresh,
    onChanged,
    onCount,
  }: {
    refresh: number;
    onChanged: () => Promise<void>;
    /** See GoogleDraftInbox: the rail renders the badge, the section owns the window. */
    onCount: (count: number) => void;
  } = $props();

  function dateKey(date: Date): string {
    return [date.getFullYear(), String(date.getMonth() + 1).padStart(2, "0"), String(date.getDate()).padStart(2, "0")].join("-");
  }

  function plusDays(date: Date, days: number): string {
    const next = new Date(date);
    next.setDate(next.getDate() + days);
    return dateKey(next);
  }

  const today = new Date();
  const from = dateKey(today);
  const to = plusDays(today, 90);
  let proposals = $state<CalendarEntry[]>([]);
  let proposedKinds = $state<Record<string, string>>({});
  let loading = $state(false);
  let actingId = $state<string | null>(null);
  let error = $state("");
  let seenRefresh = -1;

  interface CommsProposalEvidence {
    importance: "low" | "medium" | "high";
    importance_rationale: string;
    evidence: string | null;
    data_class: "c0" | "c1" | "c2" | "c3";
  }

  function dataClassLabel(dataClass: CommsProposalEvidence["data_class"]): string {
    if (dataClass === "c3") return "Secret";
    if (dataClass === "c2") return "Others";
    if (dataClass === "c1") return "Mine";
    return "Public";
  }

  function commsEvidence(entry: CalendarEntry): CommsProposalEvidence | null {
    const payload = entry.payload;
    if (!payload || typeof payload !== "object" || Array.isArray(payload)) return null;
    const value = payload as Record<string, unknown>;
    if (
      value.schema_version !== "calendar-proposal-provenance-v1" ||
      !["low", "medium", "high"].includes(String(value.importance)) ||
      typeof value.importance_rationale !== "string" ||
      !["c0", "c1", "c2", "c3"].includes(String(value.data_class))
    ) return null;
    return {
      importance: value.importance as CommsProposalEvidence["importance"],
      importance_rationale: value.importance_rationale,
      evidence: typeof value.evidence === "string" ? value.evidence : null,
      data_class: value.data_class as CommsProposalEvidence["data_class"],
    };
  }

  function proposalPriority(entry: CalendarEntry): number {
    const importance = commsEvidence(entry)?.importance;
    return importance === "high" ? 3 : importance === "medium" ? 2 : importance === "low" ? 1 : 0;
  }

  async function load() {
    loading = true;
    error = "";
    try {
      proposals = (await calendar.proposals(from, to)).sort(
        (left, right) => proposalPriority(right) - proposalPriority(left)
          || left.starts_at.localeCompare(right.starts_at),
      );
    } catch (cause) {
      error = String(cause);
    } finally {
      loading = false;
    }
    onCount(proposals.length);
  }

  $effect(() => {
    if (refresh === seenRefresh) return;
    seenRefresh = refresh;
    void load();
  });

  function kindFor(entry: CalendarEntry): string {
    return proposedKinds[entry.id] ?? entry.kind;
  }

  function setKind(entry: CalendarEntry, kind: string) {
    proposedKinds = { ...proposedKinds, [entry.id]: kind };
  }

  function when(entry: CalendarEntry): string {
    return entry.all_day ? `${entry.starts_at.slice(0, 10)} (all day)` : `${entry.starts_at} – ${entry.ends_at}`;
  }

  async function adopt(entry: CalendarEntry) {
    actingId = entry.id;
    error = "";
    try {
      await calendar.entries.update(entry.id, { kind: kindFor(entry), commitment: "planned" });
      await Promise.all([load(), onChanged()]);
    } catch (cause) {
      error = String(cause);
    } finally {
      actingId = null;
    }
  }

  async function remove(entry: CalendarEntry) {
    if (!window.confirm(`Remove “${entry.title}” from Sjel only? The source remains unchanged.`)) return;
    actingId = entry.id;
    error = "";
    try {
      await calendar.entries.delete(entry.id);
      await Promise.all([load(), onChanged()]);
    } catch (cause) {
      error = String(cause);
    } finally {
      actingId = null;
    }
  }
</script>

<p class="hint">Individual external events, whether or not they ever become a trip.</p>

{#if error}<p class="message error" role="alert">{error}</p>{/if}

{#if loading}
  <p class="empty">Loading proposals…</p>
{:else if proposals.length === 0}
  <p class="empty">Nothing pending in the next 90 days.</p>
{:else}
  <div class="proposal-list">
    {#each proposals as entry (entry.id)}
      {@const evidence = commsEvidence(entry)}
      <article class="proposal">
        <div class="details">
          <strong>{entry.title}</strong>
          <p>{when(entry)}{#if entry.location} · {entry.location}{/if}</p>
          <p class="source">Source: {entry.source}</p>
          {#if evidence}
            <p class="analysis-meta {evidence.importance}">{evidence.importance} importance · {dataClassLabel(evidence.data_class)}</p>
            <p class="analysis-rationale">{evidence.importance_rationale}</p>
            {#if evidence.evidence}<p class="analysis-evidence">Evidence: {evidence.evidence}</p>{/if}
          {/if}
        </div>
        <label>
          <span>As</span>
          <select value={kindFor(entry)} onchange={(event) => setKind(entry, event.currentTarget.value)} disabled={actingId !== null}>
            {#each KINDS as kind}<option value={kind.value}>{kind.label}</option>{/each}
          </select>
        </label>
        <div class="actions">
          <button class="btn btn-primary" onclick={() => adopt(entry)} disabled={actingId !== null}>{actingId === entry.id ? "Adding…" : "Add"}</button>
          <button class="btn btn-danger" onclick={() => remove(entry)} disabled={actingId !== null}>Remove</button>
        </div>
      </article>
    {/each}
  </div>
{/if}

<style>
  .hint { margin: 0 0 0.5rem; color: var(--text-secondary); font-size: var(--text-xs); line-height: 1.45; }
  .message { margin: 0 0 0.5rem; padding: 0.5rem 0.6rem; border-radius: var(--radius-sm); font-size: 0.78rem; }
  .error { color: var(--danger); background-color: var(--danger-soft); }
  .empty { margin: 0; color: var(--text-tertiary); font-size: 0.78rem; }
  .proposal-list { display: flex; flex-direction: column; }
  .proposal { padding: 0.6rem 0; border-top: 1px solid var(--card-border); }
  .details { min-width: 0; }
  .details strong { display: block; font-size: var(--text-sm); font-weight: 600; }
  .details p { margin: 0.15rem 0 0; color: var(--text-secondary); font-size: 0.72rem; }
  .details .source { color: var(--text-tertiary); }
  .details .analysis-meta { margin-top: 0.35rem; font-weight: 600; text-transform: capitalize; }
  .details .analysis-meta.high { color: var(--warning-ink); }
  .details .analysis-rationale,
  .details .analysis-evidence { line-height: 1.4; }
  .details .analysis-evidence { color: var(--text-tertiary); }
  label { display: flex; align-items: center; gap: 0.4rem; margin-top: 0.45rem; color: var(--text-secondary); font-size: 0.72rem; }
  select { flex: 1; min-width: 0; padding: 0.25rem 0.35rem; border: 1px solid var(--input-border); border-radius: var(--radius-sm); background-color: var(--input-bg); color: var(--text-primary); font: inherit; font-size: 0.72rem; }
  .actions { display: flex; gap: 0.3rem; margin-top: 0.45rem; }
  .actions .btn { flex: 1; padding: 0.3rem 0.5rem; font-size: 0.72rem; }
</style>
