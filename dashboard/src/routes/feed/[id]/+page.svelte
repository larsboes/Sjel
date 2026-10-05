<script lang="ts">
  import { link } from "$lib/nav";
  import { tip } from "$lib/tip";
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import EvaluationBreakdown from "$lib/feed/EvaluationBreakdown.svelte";
  import {
    loadFeedEntry,
    normalizeContentItemDetail,
    type FeedEntryLoadPhase,
  } from "$lib/feed/entry-loader";
  import MarkdownDocument from "$lib/feed/MarkdownDocument.svelte";
  import MermaidDiagram from "$lib/feed/MermaidDiagram.svelte";
  import ChartFigure from "$lib/feed/ChartFigure.svelte";
  import {
    KINDS,
    whenError,
    whenOf,
    whenPatch,
    type EntryWhen,
  } from "$lib/calendar/types";
  import {
    cloudCalendarCandidates,
    type CloudCalendarCandidate,
  } from "$lib/feed/cloud-calendar";
  import Icon from "$lib/Icon.svelte";
  import { modal } from "$lib/modal";
  import { assistantStore } from "$lib/assistant/assistant.svelte";
  import {
    ApiError,
    axonStatus,
    calendar,
    comms,
    contentItem,
    type CloudDerivativePreview,
    type CloudProvider,
    type CloudProviderUnavailableReason,
    type CalendarCommitment,
    type CalendarContentExtension,
    type CalendarUpdateEntry,
    type ContentDigest,
    type ContentItemDetail,
    type ContentSource,
    type DataClass,
    type FeedStatus,
    type MailCategory,
  } from "$lib/api";
  import { feedPersonalization } from "$lib/feed/api";

  type GmailAction = "archive" | "trash" | "restore";

  const KIND_LABEL: Record<string, string> = {
    youtube: "YouTube",
    instagram: "Instagram",
    podcast: "Podcast",
    article: "Article",
    mail: "Mail",
    github: "GitHub",
    arxiv: "arXiv",
    reddit: "Reddit",
  };

  let entry = $state<ContentItemDetail | null>(null);
  let loading = $state(true);
  let loadingMessage = $state("Loading entry…");
  let error = $state<string | null>(null);
  let busy = $state(false);
  let mailCategory = $state<MailCategory>("aktiv");
  let redactionNotice = $state<string | null>(null);
  let selectedDataClass = $state<DataClass>("c1");
  let confirmingGmailAction = $state<GmailAction | null>(null);
  let cloudPreview = $state<CloudDerivativePreview | null>(null);
  let preparingCloudPreview = $state(false);
  let approvingCloudPreview = $state(false);
  let cloudPreviewError = $state<string | null>(null);
  let cloudProviders = $state<CloudProvider[]>([]);
  let selectedProviderRole = $state("");
  let providerListLoaded = $state(false);
  let queueingCloudDerivative = $state(false);
  let runningCloudJob = $state(false);
  /// Which digest press is in flight, or null. One variable rather than three
  /// booleans: the three presses share a model and must not run at once.
  let digestBusy = $state<"standard" | "detailed" | "diagram" | "chart" | null>(null);
  let digestError = $state<string | null>(null);
  let focusInput = $state("");
  let calendarProposalBusy = $state<string | null>(null);
  let calendarProposalSaved = $state<Set<string>>(new Set());
  let calendarProposalError = $state<string | null>(null);
  let savingField = $state(false);
  /// Local drafts so a field shows what is being typed rather than snapping
  /// back to the stored value on every keystroke. Reset whenever the entry
  /// reloads, so an edit made elsewhere is not silently overwritten by a stale
  /// buffer sitting in this tab.
  let titleDraft = $state("");
  let locationDraft = $state("");
  let notesDraft = $state("");
  let whenDraft = $state<EntryWhen>({
    allDay: false,
    startDate: "",
    endDate: "",
    startTime: "",
    endTime: "",
  });
  /// Why the store would refuse this, shown next to the fields rather than as a
  /// banner after a failed round trip.
  let whenProblem = $state<string | null>(null);
  let googleExported = $state(false);
  let updatingExport = $state(false);
  const gmailRecoveryPending = $derived(
    entry?.mail?.gmail_sync_status === "queued" || entry?.mail?.gmail_sync_status === "retrying",
  );
  const gmailActionBlocked = $derived(
    gmailRecoveryPending || entry?.mail?.gmail_sync_status === "attention",
  );

  const source = $derived<ContentSource>(
    ((): ContentSource => {
      const requested = page.url.searchParams.get("source");
      return requested === "mail" || requested === "calendar" ? requested : "feed";
    })(),
  );
  const BACK: Record<ContentSource, { href: string; label: string }> = {
    feed: { href: "/feed", label: "Back to Feed" },
    mail: { href: "/feed?view=mail", label: "Back to Mail" },
    calendar: { href: "/calendar", label: "Back to Calendar" },
  };
  const backHref = $derived(BACK[source].href);
  const backLabel = $derived(BACK[source].label);
  const calendarCandidates = $derived(
    entry?.cloud_processing.result && entry.cloud_processing.job_id
      ? cloudCalendarCandidates({
          source: entry.source,
          itemId: entry.id,
          jobId: entry.cloud_processing.job_id,
          dataClass: entry.data_class.value,
          result: entry.cloud_processing.result,
        })
      : [],
  );

  /// The digest is the short version when there is one; `summary` is what the
  /// source itself said, and stays visible beside it rather than under it.
  const digestText = $derived(
    entry?.digest?.state === "generated" ? entry.digest.text : null,
  );

  const SHAPE_LABEL: Record<string, string> = {
    brief: "Brief",
    standard: "Standard",
    sectioned: "Sectioned",
    none: "None",
  };

  /// The rung, and whether it was asked for. Both, because "Sectioned" alone
  /// does not say whether the length earned it or the operator did.
  const digestRungLabel = $derived(
    entry?.digest
      ? `${SHAPE_LABEL[entry.digest.shape] ?? entry.digest.shape}${
          entry.digest.depth === "detailed" ? " · you asked for more" : ""
        }`
      : "",
  );

  /// Why there is no text. A skip is a claim about the source, so it reads
  /// differently from a model that could not be reached.
  function digestExplanation(digest: ContentDigest): string {
    switch (digest.state) {
      case "generated":
        return "";
      case "skipped_short":
        return "Too short to be worth a digest — the source is already the summary. Press More detail to force one.";
      case "remote_refused":
        return "This item is not Public and the configured model is not local, so nothing was sent.";
      // Not "could not be reached": no model was asked. Reclassifying the item
      // is the only thing that changes this answer.
      case "local_refused":
        return "Secret content enters no prompt, so nothing was sent to any model.";
      case "unconfigured":
        return "No summarization model is configured on this machine.";
      case "timeout":
        return "The local model did not answer in time.";
      case "capacity_aborted":
        // About the machine, not the model. The server accepted this request
        // and ran out of memory part-way through; the same item digests fine
        // once something stops holding the GPU, and the drain will try again.
        return "The local server ran out of memory part-way through this one. It will retry on its own.";
      case "empty_response":
        return "The local model answered with nothing.";
      default:
        return digest.last_error ?? "The local model could not be reached.";
    }
  }

  /// The diagram's own failure, which is a different axis from the digest's:
  /// a model that produced prose instead of Mermaid did answer.
  function diagramExplanation(digest: ContentDigest): string {
    if (digest.diagram_error) return digest.diagram_error;
    return digestExplanation({
      ...digest,
      state: (digest.diagram_state ?? "model_error") as ContentDigest["state"],
    });
  }

  function focusTerms(): string[] {
    return focusInput
      .split(",")
      .map((term) => term.trim())
      .filter(Boolean);
  }

  async function runDigest(depth: "standard" | "detailed") {
    if (!entry || digestBusy) return;
    digestBusy = depth;
    digestError = null;
    try {
      const digest = await comms.digest(entry.source, entry.id, { depth, focus: focusTerms() });
      entry.digest = digest;
      // What the model was asked for is now stored; showing it back from the
      // row rather than the input keeps the two from drifting after a reload.
      focusInput = digest.focus.join(", ");
    } catch (cause) {
      digestError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      digestBusy = null;
    }
  }

  async function runDiagram() {
    if (!entry || digestBusy) return;
    digestBusy = "diagram";
    digestError = null;
    try {
      entry.digest = await comms.diagram(entry.source, entry.id);
    } catch (cause) {
      digestError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      digestBusy = null;
    }
  }

  async function runChart() {
    if (!entry || digestBusy) return;
    digestBusy = "chart";
    digestError = null;
    try {
      entry.digest = await comms.chart(entry.source, entry.id);
    } catch (cause) {
      digestError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      digestBusy = null;
    }
  }

  const title = $derived(entry?.title ?? entry?.url ?? "Feed entry");
  const bodyLabel = $derived(entry?.content_label ?? "Source content");
  const readMinutes = $derived(
    Math.max(
      1,
      Math.ceil(
        `${entry?.summary ?? ""} ${entry?.content ?? ""}`.trim().split(/\s+/).filter(Boolean)
          .length / 220,
      ),
    ),
  );
  const transcriptCollapsible = $derived(
    Boolean(entry?.summary) &&
      (entry?.kind === "youtube" ||
        entry?.kind === "podcast" ||
        entry?.kind === "instagram"),
  );
  const STAGE_LABEL: Record<string, string> = {
    extraction: "Extraction",
    normalization: "Normalization",
    summary: "Summary",
    ranking: "Ranking",
  };

  /** Presentation hint only. An unknown kind falls back rather than dropping
   *  the link — the contract keeps `kind` open for exactly that reason. */
  // `as const` so the values stay literals Icon's own name union accepts —
  // Icon does not export that type, and widening this to string would push the
  // failure to runtime instead of the check.
  const LINK_ICON = {
    mail: "feed",
    source: "external",
    ticket: "ticket",
    map: "map-pin",
    vault: "boxes",
  } as const;

  function linkIcon(kind: string) {
    return LINK_ICON[kind as keyof typeof LINK_ICON] ?? "external";
  }

  const SOURCE_LABEL: Record<ContentSource, string> = {
    feed: "Feed",
    mail: "Gmail",
    calendar: "Calendar",
  };

  /** What was done to produce the body on screen. Calendar does none of it —
   *  the note is the operator's own text, stored verbatim — and saying so is
   *  more honest than borrowing feed's "Formatted source". */
  function processingLabel(item: ContentItemDetail): string {
    // "Bounded preview" alone stopped being true for mail the moment a digest
    // could read the full message: the body on screen is still the snippet, but
    // the digest is not made from it, and a label that hides that is the kind
    // of quiet understatement this reader exists to avoid.
    const digested = item.digest?.state === "generated";
    if (item.source === "calendar") {
      if (digested) return "Digest + your note";
      return item.summary ? "Summary + your note" : "Your note";
    }
    if (item.source === "mail") return digested ? "Digest + bounded preview" : "Bounded preview";
    if (digested) return "Digest + source";
    if (item.summary) return "Summary + source";
    return "Formatted source";
  }

  const COMMITMENT_LABEL: Record<string, string> = {
    possible: "On the radar",
    planned: "Planned",
    committed: "Committed",
  };

  /// Editing a calendar entry from the reader.
  ///
  /// Everything the calendar form edits except delete and the Google export
  /// opt-in: what it is, when it happens, how binding it is, and the text.
  /// The date rules are not reimplemented here — `whenOf`/`whenPatch`/`whenError`
  /// in `calendar/types.ts` are the single implementation, and the form was
  /// moved onto them at the same time so the two cannot drift apart.
  async function patchEntry(patch: CalendarUpdateEntry): Promise<void> {
    if (!entry || entry.source !== "calendar" || savingField) return;
    savingField = true;
    error = null;
    try {
      const updated = await calendar.entries.update(entry.id, patch);
      // Re-derive from what the store actually returned, never from the patch:
      // a PATCH detaches a rhythm-linked entry, and the response is the only
      // place that shows up.
      entry = {
        ...entry,
        kind: updated.kind,
        title: updated.title,
        status: updated.commitment,
        content: updated.notes,
        content_status: updated.notes ? "full" : "none",
        calendar: {
          starts_at: updated.starts_at,
          ends_at: updated.ends_at,
          all_day: updated.all_day,
          commitment: updated.commitment,
          location: updated.location,
          notes: updated.notes,
          entry_source: updated.source,
          // From the response, never carried over: a patch detaches a
          // rhythm-linked entry, and this is where that becomes visible.
          rhythm_id: updated.rhythm_id,
        },
      };
      // Re-seed from the stored values: toggling all-day rewrites both ends, and
      // the fields have to show what was actually saved rather than what was typed.
      whenDraft = whenOf(updated);
      whenProblem = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      savingField = false;
    }
  }

  /// Why exporting this entry to Google would be dishonest, if it would.
  ///
  /// Same two rules the calendar form applies — the reader can only enforce them
  /// because the calendar extension carries `entry_source` and `rhythm_id`.
  const exportBlocked = $derived.by(() => {
    const extension = entry?.calendar;
    if (!extension) return null;
    if (extension.entry_source === "google") {
      return "This entry came from Google and will not be exported back.";
    }
    if (extension.rhythm_id) {
      return "Rhythm instances are not exported to Google individually.";
    }
    return null;
  });

  async function toggleGoogleExport(): Promise<void> {
    if (!entry || exportBlocked || updatingExport) return;
    const next = !googleExported;
    updatingExport = true;
    error = null;
    try {
      if (next) await calendar.google.optInExport(entry.id);
      else await calendar.google.optOutExport(entry.id);
      googleExported = next;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      updatingExport = false;
    }
  }

  /// Deleting removes Sjel's copy only. Any Google event this entry already
  /// created is deliberately left alone — the capability's own opt-out route
  /// makes the same call, and removing someone's calendar event as a side
  /// effect of a delete here is not a decision this page makes.
  async function deleteEntry(): Promise<void> {
    if (!entry?.calendar) return;
    if (!window.confirm(`Delete “${entry.title}” from Sjel? Any Google event it created stays.`)) {
      return;
    }
    savingField = true;
    error = null;
    try {
      await calendar.entries.delete(entry.id);
      await goto(link("/calendar"));
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
      savingField = false;
    }
  }

  /// Commits a date/time change through the same conversion the calendar form
  /// uses (`whenPatch`), so the two surfaces cannot disagree about which day an
  /// all-day entry ends on — the inclusive/exclusive mismatch is the one bug
  /// worth centralising.
  function commitWhen(next: Partial<EntryWhen>): void {
    if (!entry?.calendar) return;
    const merged: EntryWhen = { ...whenDraft, ...next };
    whenDraft = merged;
    whenProblem = whenError(merged);
    if (whenProblem) return;
    const patch = whenPatch(merged);
    const current = entry.calendar;
    if (
      patch.starts_at === current.starts_at &&
      patch.ends_at === current.ends_at &&
      patch.all_day === current.all_day
    ) {
      return;
    }
    void patchEntry(patch);
  }

  /// Saves only when the value actually changed, so tabbing through a field
  /// does not write a no-op revision.
  function commitText(field: "title" | "location" | "notes", value: string): void {
    if (!entry?.calendar) return;
    const trimmed = value.trim();
    const current =
      field === "title" ? (entry.title ?? "") : (entry.calendar[field] ?? "");
    if (trimmed === current.trim()) return;
    if (field === "title") {
      // The store rejects an empty title, so an emptied field reverts rather
      // than sending a patch that is going to 400.
      if (!trimmed) {
        titleDraft = entry.title ?? "";
        return;
      }
      void patchEntry({ title: trimmed });
      return;
    }
    // Location and notes are genuinely nullable — clearing them means "none",
    // not "unchanged", which is why UpdateEntry models them as present-nullable.
    void patchEntry({ [field]: trimmed || null } as CalendarUpdateEntry);
  }

  function phaseMessage(phase: FeedEntryLoadPhase): string {
    if (phase === "starting") return "Starting Feed…";
    if (phase === "retrying") return "Opening entry…";
    return "Loading entry…";
  }

  async function loadEntry(): Promise<void> {
    loading = true;
    error = null;
    try {
      entry = normalizeContentItemDetail(await loadFeedEntry({
        id: page.params.id ?? "",
        // One contract and one route shape, so the reader resolves an item from
        // its source alone — no per-capability branch here at all.
        read: (id, signal) => contentItem(source, id, signal),
        start: (signal) => axonStatus.start(source === "calendar" ? "calendar" : "comms", signal),
        shouldRetry: (cause) => !(cause instanceof ApiError && cause.status === 404),
        onPhase: (phase) => { loadingMessage = phaseMessage(phase); },
      }));
      if (entry.mail) {
        mailCategory = entry.mail.category;
      }
      selectedDataClass = entry.data_class.value;
      titleDraft = entry.title ?? "";
      locationDraft = entry.calendar?.location ?? "";
      notesDraft = entry.calendar?.notes ?? "";
      if (entry.calendar) {
        whenDraft = whenOf(entry.calendar);
        void loadGoogleExportState(entry.id);
      }
      whenProblem = null;
      // After the entry resolves, and only for a feed item. The same route
      // serves mail and calendar, so an ungated POST would send a triage id to
      // /feed/:id/interactions and swallow a 404 on every mail open. An entry
      // the operator already decided on is a REOPEN: coming back to something
      // filed is a different signal from a first read.
      //
      // In-app entry-to-entry navigation records nothing, and that is stated
      // rather than silently missed: `loadEntry` has two callers and does not
      // re-run on a parameter change, so this page never observes one.
      if (entry.source === "feed") {
        void feedPersonalization.recordInteraction(
          entry.id,
          entry.status === "new" ? "opened" : "reopened",
          "reader",
        );
      }
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      loading = false;
    }
  }

  /// The opt-in ledger is the source of truth for whether this entry exports;
  /// the entry itself does not carry the flag. A failure here leaves the toggle
  /// off rather than surfacing an error — not knowing is not the same as
  /// something being wrong, and the export page reports ledger problems.
  async function loadGoogleExportState(entryId: string): Promise<void> {
    try {
      const optIns = await calendar.google.exports();
      googleExported = optIns.some((optIn) => optIn.entry_id === entryId);
    } catch {
      googleExported = false;
    }
  }

  async function loadCloudProviders(): Promise<void> {
    try {
      cloudProviders = await comms.cloudProviders();
      const queuedRole = entry?.cloud_processing.provider_role;
      selectedProviderRole = cloudProviders.some((provider) => provider.role === queuedRole)
        ? queuedRole ?? ""
        : cloudProviders.find((provider) => provider.available)?.role ?? "";
    } catch (cause) {
      cloudPreviewError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      providerListLoaded = true;
    }
  }

  onMount(() => {
    void loadEntry();
    void loadCloudProviders();
  });

  async function setStatus(status: FeedStatus): Promise<void> {
    if (!entry || entry.source !== "feed" || busy) return;
    busy = true;
    try {
      await feedPersonalization.setStatus(entry.id, status, "reader");
      entry.status = status;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  /** The two categories that raise a mail's data class by name alone, so
   *  setting one also permanently redacts the stored subject and preview.
   *  `content_item::mail_others_reason` is what rules it in comms. */
  const CLASS_RAISING_CATEGORIES: MailCategory[] = ["belege", "steuern"];

  async function setMailCategory(category: MailCategory): Promise<void> {
    if (!entry?.mail || entry.source !== "mail" || busy || category === entry.mail.category) return;
    // Asked before the write, because the write cannot be undone: moving a mail
    // to Tax or Receipts raises it to Others, and the stored subject and preview
    // are redacted in the same transaction.
    if (
      CLASS_RAISING_CATEGORIES.includes(category) &&
      !window.confirm(
        "Tax and Receipts are Others: this also raises the data class and permanently redacts the stored subject and preview. A later sweep cannot put them back. Continue?",
      )
    ) {
      mailCategory = entry.mail.category;
      return;
    }
    busy = true;
    error = null;
    redactionNotice = null;
    try {
      const write = await comms.setTriageCategory(entry.id, category);
      if (write.narrowed) {
        redactionNotice =
          "The stored subject and preview were permanently redacted: the class this category sets does not admit them.";
        // Reloaded rather than patched, because the row on screen still holds
        // the text the write just removed.
        await loadEntry();
        return;
      }
      entry.mail.category = category;
      entry.mail.rationale = "Category set manually in Sjel.";
      entry.mail.classification_method = "human";
      entry.mail.classification_version = "manual-v1";
    } catch (cause) {
      mailCategory = entry.mail.category;
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function setMailStatus(status: "proposed" | "approved" | "dismissed"): Promise<void> {
    if (!entry?.mail || entry.source !== "mail" || busy) return;
    busy = true;
    error = null;
    try {
      await comms.setTriageStatus(entry.id, status);
      entry.status = status;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function setDataClass(dataClass: DataClass): Promise<void> {
    if (!entry?.mail || entry.source !== "mail" || busy || dataClass === entry.data_class.value) return;
    busy = true;
    error = null;
    try {
      const id = entry.id;
      await comms.setTriageDataClass(id, dataClass);
      entry = normalizeContentItemDetail(await comms.content("mail", id));
      selectedDataClass = entry.data_class.value;
    } catch (cause) {
      selectedDataClass = entry.data_class.value;
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      busy = false;
    }
  }

  async function applyGmailAction(action: GmailAction): Promise<void> {
    if (!entry?.mail || entry.source !== "mail" || busy) return;
    const id = entry.id;
    busy = true;
    error = null;
    try {
      await comms.applyGmailAction(id, action);
      entry = normalizeContentItemDetail(await comms.content("mail", id));
      mailCategory = entry.mail?.category ?? mailCategory;
      confirmingGmailAction = null;
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
      try {
        entry = normalizeContentItemDetail(await comms.content("mail", id));
      } catch {
        error = `${error} Sjel could not refresh the queued recovery state.`;
      }
    } finally {
      busy = false;
    }
  }

  async function decideGmailJob(decision: "retry" | "cancel"): Promise<void> {
    if (!entry?.mail || entry.source !== "mail" || busy) return;
    const id = entry.id;
    busy = true;
    error = null;
    try {
      await comms.decideGmailJob(id, decision);
    } catch (cause) {
      error = cause instanceof Error ? cause.message : String(cause);
    } finally {
      try {
        entry = normalizeContentItemDetail(await comms.content("mail", id));
        mailCategory = entry.mail?.category ?? mailCategory;
      } catch {
        error = error ? `${error} Sjel could not refresh the recovery state.` : "Sjel could not refresh the recovery state.";
      }
      busy = false;
    }
  }

  function dataClassLabel(value: DataClass): string {
    if (value === "c3") return "Secret";
    if (value === "c2") return "Others";
    if (value === "c1") return "Mine";
    return "Public";
  }

  function lifecycleDate(value: string | null): string {
    if (!value) return "—";
    const date = new Date(value);
    return Number.isNaN(date.getTime())
      ? "—"
      : new Intl.DateTimeFormat("en-GB", { dateStyle: "medium", timeStyle: "short" }).format(date);
  }

  async function prepareCloudPreview(): Promise<void> {
    if (!entry || preparingCloudPreview || approvingCloudPreview) return;
    preparingCloudPreview = true;
    cloudPreviewError = null;
    try {
      cloudPreview = await comms.prepareCloudPreview(entry.source, entry.id);
    } catch (cause) {
      cloudPreviewError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      preparingCloudPreview = false;
    }
  }

  async function approveCloudPreview(): Promise<void> {
    if (!entry || !cloudPreview || approvingCloudPreview) return;
    approvingCloudPreview = true;
    cloudPreviewError = null;
    try {
      entry.cloud_processing = await comms.approveCloudPreview(
        entry.source,
        entry.id,
        cloudPreview.preview_hash,
      );
      cloudPreview = null;
    } catch (cause) {
      cloudPreviewError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      approvingCloudPreview = false;
    }
  }

  async function queueCloudDerivative(): Promise<void> {
    if (
      !entry?.cloud_processing.preview_hash ||
      !selectedProviderRole ||
      queueingCloudDerivative
    ) return;
    queueingCloudDerivative = true;
    cloudPreviewError = null;
    try {
      entry.cloud_processing = await comms.queueCloudDerivative(
        entry.source,
        entry.id,
        entry.cloud_processing.preview_hash,
        selectedProviderRole,
      );
    } catch (cause) {
      cloudPreviewError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      queueingCloudDerivative = false;
    }
  }

  /// One sentence per reason the server can refuse a role. The roster used to
  /// collapse all four into "setup required", which read as "you forgot a key"
  /// even when the real cause was an exhausted daily budget.
  const PROVIDER_UNAVAILABLE_LABEL: Record<CloudProviderUnavailableReason, string> = {
    missing_credential: "No API key materialized on this machine",
    billing_expired_or_unknown: "Billing window expired or unknown",
    budget_unavailable: "Today's call ledger could not be read",
    daily_request_limit_reached: "Daily request limit reached",
  };

  /// `available` is a state, not a reason, so it gets its own branch rather
  /// than a fifth entry in the map.
  function providerStateLabel(provider: CloudProvider): string {
    if (provider.available) return "Available";
    return provider.unavailable_reason === null
      ? "Unavailable, with no reason reported"
      : PROVIDER_UNAVAILABLE_LABEL[provider.unavailable_reason];
  }

  /// Remaining budget, keeping "the ledger did not answer" distinct from a
  /// genuine zero — the server reports the first as a null, not as 0 used.
  function providerBudgetLabel(provider: CloudProvider): string {
    const limit = `${provider.max_requests_per_day}/day`;
    if (provider.requests_remaining_today === null || provider.requests_used_today === null) {
      return `Unknown · ${limit} configured`;
    }
    return `${provider.requests_remaining_today} left · ${provider.requests_used_today} used · ${limit}`;
  }

  function providerTierLabel(tier: CloudProvider["data_tier"]): string {
    return tier === "pseudonymized_personal"
      ? "Reviewed Personal derivatives"
      : "Public derivatives only";
  }

  function providerBillingLabel(provider: CloudProvider): string {
    const mode = provider.billing_mode === "free_only" ? "Free tier only" : "Prepaid credit";
    return provider.credit_expires_on
      ? `${mode} · credit expires ${provider.credit_expires_on}`
      : `${mode} · no credit expiry`;
  }

  function cloudDispatchLabel(status: ContentItemDetail["cloud_processing"]["dispatch_status"]): string {
    if (status === "queued") return "Ready to run";
    if (status === "running") return "Running";
    if (status === "succeeded") return "Completed";
    if (status === "failed") return "Needs attention";
    return entry?.cloud_processing.status === "staged" ? "Staged locally" : "Not prepared";
  }

  async function runCloudJob(): Promise<void> {
    const jobId = entry?.cloud_processing.job_id;
    if (!entry || !jobId || runningCloudJob) return;
    const source = entry.source;
    const itemId = entry.id;
    runningCloudJob = true;
    cloudPreviewError = null;
    try {
      entry.cloud_processing = await comms.runCloudJob(jobId);
    } catch (cause) {
      cloudPreviewError = cause instanceof Error ? cause.message : String(cause);
      try {
        entry = normalizeContentItemDetail(await comms.content(source, itemId));
      } catch {
        cloudPreviewError = `${cloudPreviewError} Sjel could not refresh the job state.`;
      }
    } finally {
      runningCloudJob = false;
    }
  }

  async function proposeToCalendar(candidate: CloudCalendarCandidate): Promise<void> {
    if (calendarProposalBusy) return;
    calendarProposalBusy = candidate.key;
    calendarProposalError = null;
    try {
      await calendar.entries.upsertExternal(candidate.entry);
      calendarProposalSaved = new Set([...calendarProposalSaved, candidate.key]);
    } catch (cause) {
      calendarProposalError = cause instanceof Error ? cause.message : String(cause);
    } finally {
      calendarProposalBusy = null;
    }
  }
</script>

<svelte:head>
  <title>{title} · Sjel Feed</title>
</svelte:head>

<a class="back" href={backHref}>← {backLabel}</a>

{#if loading}
  <p class="state" aria-live="polite"><Icon name="loader" size={14} /> {loadingMessage}</p>
{:else if error && !entry}
  <div class="state error" role="alert">
    <span>{error}</span>
    <button class="btn" type="button" onclick={() => void loadEntry()}>Try again</button>
  </div>
{:else if entry}
  <article>
    <div class="reader-grid">
      <main class="reader">
        <header class="article-head">
      <div class="overline">
        <span class="tag mono">{KIND_LABEL[entry.kind] ?? entry.kind}</span>
        <span>{new Date(entry.created_at).toLocaleDateString("en-GB")}</span>
        {#if entry.source !== "calendar"}
          <span class="read-pill"><Icon name="clock" size={11} /> {readMinutes} min read</span>
        {/if}
      </div>
      {#if entry.calendar}
        <!-- Looks like the heading until you focus it. A calendar entry is
             yours to rename; a feed article's title belongs to its publisher,
             which is why only this branch is editable. -->
        <input
          class="title-edit"
          aria-label="Entry title"
          bind:value={titleDraft}
          disabled={savingField}
          onblur={() => commitText("title", titleDraft)}
          onkeydown={(event) => {
            if (event.key === "Enter") event.currentTarget.blur();
            if (event.key === "Escape") titleDraft = entry?.title ?? "";
          }}
        />
      {:else}
        <h1>{title}</h1>
      {/if}
      {#if entry.author}<p class="byline">{entry.author}</p>{/if}

      <!-- When it happens and how binding it is — the two facts a calendar
           item leads with, where a feed article leads with its byline. -->
      {#if entry.calendar}
        <p class="when">
          <Icon name="calendar" size={14} />
          <!-- Native date/time inputs, so the OS picker and keyboard entry both
               work; the exclusive-end and all-day rules stay in whenPatch. -->
          <span class="when-fields">
            <input
              type="date"
              aria-label="Start date"
              value={whenDraft.startDate}
              disabled={savingField}
              onchange={(event) => commitWhen({ startDate: event.currentTarget.value })}
            />
            {#if !whenDraft.allDay}
              <input
                type="time"
                aria-label="Start time"
                value={whenDraft.startTime}
                disabled={savingField}
                onchange={(event) => commitWhen({ startTime: event.currentTarget.value })}
              />
            {/if}
            <span class="dash">–</span>
            {#if !whenDraft.allDay}
              <input
                type="time"
                aria-label="End time"
                value={whenDraft.endTime}
                disabled={savingField}
                onchange={(event) => commitWhen({ endTime: event.currentTarget.value })}
              />
            {/if}
            <!-- The end date is inclusive here, as in the form: an all-day entry
                 ending on the 16th covers the 15th, and showing the stored
                 exclusive value would report a day you are not busy. -->
            <input
              type="date"
              aria-label="End date (inclusive)"
              value={whenDraft.endDate}
              disabled={savingField}
              onchange={(event) => commitWhen({ endDate: event.currentTarget.value })}
            />
            <label class="all-day">
              <input
                type="checkbox"
                checked={whenDraft.allDay}
                disabled={savingField}
                onchange={(event) => commitWhen({ allDay: event.currentTarget.checked })}
              />
              All day
            </label>
          </span>
          <!-- The triage axis, and the calendar analogue of feed's Keep /
               Dismiss: the one decision worth making without leaving the page. -->
          <span class="commitment-set" role="group" aria-label="Commitment">
            {#each ["possible", "planned", "committed"] as level (level)}
              <button
                type="button"
                class="commitment commitment-{level}"
                class:chosen={entry.calendar.commitment === level}
                disabled={savingField}
                onclick={() => patchEntry({ commitment: level as CalendarCommitment })}
              >
                {COMMITMENT_LABEL[level]}
              </button>
            {/each}
          </span>
        </p>
        <!-- Its own row: a full street address does not fit beside four date
             controls and a commitment switch, and clipping the venue is worse
             than one more line. -->
        <p class="where-row">
          <Icon name="map-pin" size={14} />
          <input
            class="where-edit"
            aria-label="Location"
            placeholder="Add a location"
            bind:value={locationDraft}
            disabled={savingField}
            onblur={() => commitText("location", locationDraft)}
            onkeydown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
              if (event.key === "Escape") locationDraft = entry?.calendar?.location ?? "";
            }}
          />
        </p>
        {#if whenProblem}
          <p class="when-problem" role="alert">{whenProblem}</p>
        {/if}
        {#if exportBlocked}
          <p class="export-note">{exportBlocked}</p>
        {/if}
      {/if}

      <div class="actions">
        {#if entry.source === "mail" && entry.status === "missing"}
          <span class="btn missing-source" aria-disabled="true">No longer in Gmail</span>
        {:else if entry.source === "calendar"}
          <!-- Not target="_blank": a calendar item's url is a path inside this
               same dashboard, so opening a second tab would be wrong. `url`
               carries the ?entry= that makes the calendar open its edit form,
               so this is the way back to changing the entry now that clicking a
               card reads it instead. -->
          <a class="btn btn-primary" href={entry.url}>
            Open in Calendar <Icon name="arrow-right" size={13} />
          </a>
          <!-- Opting in only queues the entry; the export itself is a separate,
               explicit run on the Calendar page. Saying "approve" rather than
               "export" keeps that distinction visible. -->
          <button
            class="btn"
            class:kept={googleExported}
            type="button"
            disabled={updatingExport || savingField || Boolean(exportBlocked)}
            use:tip={"Approve this entry for the next Google export run"}
            onclick={() => void toggleGoogleExport()}
          >
            {#if updatingExport}
              <Icon name="loader" size={13} />
            {:else}
              {googleExported ? "Approved for Google" : "Approve for Google"}
            {/if}
          </button>
          <button
            class="btn danger"
            type="button"
            disabled={savingField}
            onclick={() => void deleteEntry()}
          >
            Delete
          </button>
        {:else}
          <a class="btn btn-primary" href={entry.url} target="_blank" rel="noreferrer">
            {entry.source === "mail" ? "Open in Gmail" : "Open original"} <Icon name="external" size={13} />
          </a>
        {/if}
        {#if entry.source === "feed"}
          <button
            class="btn"
            class:kept={entry.status === "keeper"}
            disabled={busy}
            onclick={() => setStatus(entry?.status === "keeper" ? "new" : "keeper")}
          >
            <Icon name="check" size={13} />
            {entry.status === "keeper" ? "Remove from saved" : "Keep"}
          </button>
          <button class="btn" disabled={busy} onclick={() => setStatus("dismissed")}>
            <Icon name="close" size={13} /> Dismiss
          </button>
          <button
            class="btn"
            type="button"
            onclick={() => {
              if (entry) {
                void assistantStore.send(`Summarize and suggest action points for "${entry.title}": ${entry.url}`, page.url.pathname);
              }
            }}
          >
            <Icon name="sparkles" size={13} /> Ask Sjel
          </button>
        {:else if entry.mail}
          <!-- "Make a task" was here until PRD Q48 (2026-08-27). The Action kind
               moved back into the vault and the vault server is read-only, so
               there is nothing for this button to POST to: a task is a note a
               human writes in Obsidian. The mail keeps its own lane — category,
               archive, trash — and the hand-off to an action is a note, not a
               row this page can create. -->
          {#if redactionNotice}<p class="mail-redaction-notice">{redactionNotice}</p>{/if}
          <label class="mail-category">
            <span>Category</span>
            <select
              bind:value={mailCategory}
              disabled={busy || entry.status === "executed" || entry.status === "archived" || entry.status === "trashed"}
              onchange={() => void setMailCategory(mailCategory)}
            >
              <option value="issue">Action</option>
              <option value="aktiv">Active</option>
              <option value="steuern">Tax</option>
              <option value="belege">Receipts</option>
              <option value="feed">Feed</option>
              <option value="sonstiges">Other</option>
              <option value="werbung">Advertising</option>
            </select>
          </label>
          {#if entry.status === "dismissed"}
            <button class="btn" disabled={busy} onclick={() => setMailStatus("proposed")}>Return to queue</button>
          {:else if entry.status === "missing"}
            <span class="lifecycle-state mono">Missing in Gmail · retained in Sjel</span>
            <button class="btn" disabled={busy} onclick={() => setMailStatus("dismissed")}>Dismiss from Sjel</button>
          {:else if entry.status === "archived" || entry.status === "trashed"}
            <span class="lifecycle-state mono">
              {entry.status === "archived" ? "Archived in Sjel + Gmail" : "In Sjel + Gmail Trash"}
            </span>
            <button class="btn" disabled={busy || gmailActionBlocked} onclick={() => applyGmailAction("restore")}>Restore to Inbox</button>
          {:else if entry.status !== "executed"}
            <button class="btn" disabled={busy} onclick={() => setMailStatus("dismissed")}>Dismiss from Sjel</button>
            <button class="btn" disabled={busy || gmailActionBlocked} onclick={() => (confirmingGmailAction = "archive")}>Archive in Sjel + Gmail</button>
            <button class="btn danger" disabled={busy || gmailActionBlocked} onclick={() => (confirmingGmailAction = "trash")}>Move to Trash</button>
          {/if}
        {/if}
      </div>

      <!-- The shared links vocabulary. Source-agnostic on purpose: a Luma event
           page, the mail that carried a ticket and a map pin all render here,
           and a source that adds a new link kind needs no change to this. -->
      {#if entry.links.length > 0}
        <ul class="links">
          {#each entry.links as link (link.url)}
            <li>
              <a
                href={link.url}
                target={link.url.startsWith("/") ? null : "_blank"}
                rel={link.url.startsWith("/") ? null : "noreferrer"}
              >
                <Icon name={linkIcon(link.kind)} size={13} />
                {link.label}
              </a>
            </li>
          {/each}
        </ul>
      {/if}

      {#if confirmingGmailAction}
        <div class="gmail-confirm" role="alert">
          <p>
            {confirmingGmailAction === "trash"
              ? "Move this thread to Trash in Sjel and Gmail? Sjel retains its copy for 30 days."
              : "Archive this thread in Sjel and remove it from the Gmail Inbox?"}
          </p>
          <button class="btn" disabled={busy} onclick={() => (confirmingGmailAction = null)}>Cancel</button>
          <button
            class="btn"
            class:danger={confirmingGmailAction === "trash"}
            disabled={busy}
            onclick={() => applyGmailAction(confirmingGmailAction!)}
          >
            {busy ? "Applying…" : confirmingGmailAction === "trash" ? "Move to Trash" : "Archive"}
          </button>
        </div>
      {/if}
      {#if entry.mail?.gmail_sync_status === "attention"}
        <div class="gmail-confirm" role="status">
          <p>
            The {entry.mail.gmail_sync_action ?? "Gmail"} action stopped after five failed attempts.
            Retry opens a fresh bounded attempt window; Cancel keeps the current Gmail and Sjel state.
          </p>
          <button class="btn" disabled={busy} onclick={() => decideGmailJob("cancel")}>Cancel action</button>
          <button class="btn btn-primary" disabled={busy} onclick={() => decideGmailJob("retry")}>Retry action</button>
        </div>
      {/if}
      {#if error}<p class="inline-error" aria-live="polite">{error}</p>{/if}
        </header>

        <section class="note digest" aria-labelledby="digest-title">
          <div class="digest-head">
            <p class="section-label" id="digest-title">Digest</p>
            {#if digestText}
              <span class="digest-rung">{digestRungLabel}</span>
            {/if}
          </div>

          {#if digestText}
            <MarkdownDocument content={digestText} compact />
          {:else if entry.digest}
            <p class="digest-note">{digestExplanation(entry.digest)}</p>
          {:else}
            <p class="digest-note">Not generated yet.</p>
          {/if}

          {#if entry.digest?.focus?.length}
            <p class="digest-focus">
              Asked to focus on: {entry.digest.focus.join(", ")}
            </p>
          {/if}
          {#if entry.digest && entry.digest.redactions > 0}
            <p class="digest-redactions">
              {entry.digest.redactions}
              {entry.digest.redactions === 1 ? "entity was" : "entities were"} redacted before this
              was stored — this item's class requires redaction, and a digest must not republish
              what the sweep removed.
            </p>
          {/if}

          <div class="digest-controls">
            <label class="digest-field">
              <span>Focus on</span>
              <input
                type="text"
                placeholder="optional — comma-separated, e.g. cost, evaluation"
                bind:value={focusInput}
                disabled={digestBusy !== null}
              />
            </label>
            <div class="digest-buttons">
              <button
                class="btn"
                disabled={digestBusy !== null}
                onclick={() => runDigest("standard")}
                use:tip={"Regenerate at the rung this source's length earns"}
              >
                {digestBusy === "standard" ? "Digesting…" : "Regenerate"}
              </button>
              <button
                class="btn btn-primary"
                disabled={digestBusy !== null}
                onclick={() => runDigest("detailed")}
                use:tip={"One rung further up the same ladder"}
              >
                {digestBusy === "detailed" ? "Digesting…" : "More detail"}
              </button>
              <button
                class="btn"
                disabled={digestBusy !== null}
                onclick={runDiagram}
                use:tip={"Draw this item as a Mermaid diagram"}
              >
                {digestBusy === "diagram" ? "Drawing…" : "Diagram"}
              </button>
              <button
                class="btn"
                disabled={digestBusy !== null}
                onclick={runChart}
                use:tip={"Pull the item's numbers out and plot them"}
              >
                {digestBusy === "chart" ? "Reading…" : "Chart"}
              </button>
            </div>
          </div>

          {#if entry.digest?.chart}
            <ChartFigure data={entry.digest.chart} />
          {:else if entry.digest?.chart_state && entry.digest.chart_state !== "generated"}
            <p class="digest-note">
              {entry.digest.chart_state === "skipped_short"
                ? "No chart: this source has no set of comparable numbers in it. Most prose does not."
                : `No chart: ${entry.digest.chart_error ?? "the local model could not be reached."}`}
            </p>
          {/if}

          {#if entry.digest?.diagram}
            <MermaidDiagram source={entry.digest.diagram} />
          {:else if entry.digest?.diagram_state && entry.digest.diagram_state !== "generated"}
            <p class="digest-note">No diagram: {diagramExplanation(entry.digest)}</p>
          {/if}

          {#if digestError}<p class="inline-error" aria-live="polite">{digestError}</p>{/if}
          {#if entry.digest}
            <p class="digest-provenance">
              {entry.digest.source_chars.toLocaleString()} characters of source · produced by
              <code>{entry.digest.producer}</code>
            </p>
          {/if}
        </section>

        <!-- `summary` is what the *source* said it is, and stays beside the
             digest rather than being replaced by it: for a calendar entry it is
             the only verbatim description there is. -->
        {#if entry.summary && entry.summary !== digestText}
          <section class="note">
            <p class="section-label">{entry.calendar ? "As described" : "Summary"}</p>
            <MarkdownDocument content={entry.summary} compact />
          </section>
        {/if}

        {#if entry.content && transcriptCollapsible}
          <details class="transcript-disclosure">
            <summary>
              <span>
                <span class="section-label">Source</span>
                <strong>{bodyLabel}</strong>
              </span>
              <span class="disclosure-meta">{readMinutes} min · open</span>
            </summary>
            <div class="transcript-body">
              <a class="source-link" href={entry.url} target="_blank" rel="noreferrer">
                Read original <Icon name="external" size={12} />
              </a>
              <MarkdownDocument content={entry.content} />
            </div>
          </details>
          <!-- `|| entry.calendar` so an entry with no note yet still renders the
             field. Without it a fresh entry fell through to "no readable source
             text", which is both wrong and the one state where you most want to
             type something. -->
        {:else if entry.content || entry.calendar}
          <section class="source-document">
            <div class="document-head">
              <div>
                <p class="section-label">{entry.calendar ? "Yours" : "Source"}</p>
                <h2>{bodyLabel}</h2>
              </div>
              <!-- "Read original" is a promise about somewhere else. A calendar
                   note has no elsewhere, so it does not offer one. -->
              {#if entry.source !== "calendar"}
                <a href={entry.url} target="_blank" rel="noreferrer">
                  Read original <Icon name="external" size={12} />
                </a>
              {/if}
            </div>
            {#if entry.calendar}
              <!-- Your own note, so it is a field rather than a rendered
                   document. Markdown still renders everywhere else, where the
                   text came from a source and is not yours to retype. -->
              <textarea
                class="notes-edit"
                aria-label="Note"
                rows="4"
                placeholder="Why this matters, what to bring, anything worth remembering."
                bind:value={notesDraft}
                disabled={savingField}
                onblur={() => commitText("notes", notesDraft)}
              ></textarea>
            {:else}
              <!-- Non-null by the branch condition, but TS cannot narrow across
                   the `|| entry.calendar` above. -->
              <MarkdownDocument content={entry.content ?? ""} />
            {/if}
          </section>
        {:else}
          <section class="source-document empty">
            No readable source text is available for this entry. The original remains linked.
          </section>
        {/if}
      </main>

      <aside class="context">
        <section>
          <p class="section-label">Entry</p>
          <dl>
            <div>
              <dt>Type</dt>
              <dd>
                {#if entry.calendar}
                  <!-- Kind drives the colour in the grid and the travel verdict
                       in the correlation layer, so it is worth changing without
                       a round trip to the form. -->
                  <select
                    class="kind-edit"
                    aria-label="Entry type"
                    value={entry.kind}
                    disabled={savingField}
                    onchange={(event) => patchEntry({ kind: event.currentTarget.value })}
                  >
                    {#each KINDS as option (option.value)}
                      <option value={option.value}>{option.label}</option>
                    {/each}
                  </select>
                {:else}
                  {KIND_LABEL[entry.kind] ?? entry.kind}
                {/if}
              </dd>
            </div>
            <div>
              <dt>Captured</dt>
              <dd>{new Date(entry.created_at).toLocaleDateString("en-GB")}</dd>
            </div>
            <!-- A reading estimate is a claim about text you have to get
                 through. A calendar entry's body is a note you wrote; the
                 number would be noise dressed as information. -->
            {#if entry.source !== "calendar"}
              <div><dt>Reading time</dt><dd>{readMinutes} min</dd></div>
            {/if}
            <div><dt>Source</dt><dd>{SOURCE_LABEL[entry.source]}</dd></div>
            <div><dt>Status</dt><dd>{entry.status}</dd></div>
            <div>
              <dt>Processing</dt>
              <dd>{processingLabel(entry)}</dd>
            </div>
          </dl>
        </section>

        <section aria-labelledby="data-handling-title">
          <p class="section-label" id="data-handling-title">Data handling</p>
          {#if entry.mail}
            <label class="data-class-control">
              <span>Data class</span>
              <select
                bind:value={selectedDataClass}
                disabled={busy}
                onchange={() => void setDataClass(selectedDataClass)}
              >
                <option value="c0">Public</option>
                <option value="c1">Mine</option>
                <option value="c2">Others</option>
                <option value="c3">Secret</option>
              </select>
            </label>
          {:else}
            <p class="data-class-value">{entry.data_class.label}</p>
          {/if}
          <dl>
            <div><dt>Local processing</dt><dd>{entry.processing_policy.local_processing}</dd></div>
            <div>
              <dt>Cloud handling</dt>
              <dd>
                {entry.processing_policy.cloud_handling === "eligible"
                  ? "Eligible"
                  : entry.processing_policy.cloud_handling === "pseudonymization_required"
                    ? "Pseudonymization required"
                    : "Blocked"}
              </dd>
            </div>
            <div><dt>Classification</dt><dd>{entry.data_class.method}</dd></div>
          </dl>
          <p class="classification-rationale">{entry.data_class.rationale}</p>
          <p class="policy-rationale">{entry.processing_policy.rationale}</p>
        </section>

        <!-- Calendar has no cloud derivative pipeline, and its items report
             `not_prepared` truthfully. Offering "Preview cloud-ready copy"
             anyway would advertise a button that cannot do anything. -->
        {#if entry.source !== "calendar"}
        <section class="cloud-processing" aria-labelledby="cloud-processing-title">
          <div class="aside-title">
            <p class="section-label" id="cloud-processing-title">Cloud processing</p>
            <span class:staged={entry.cloud_processing.status === "staged"}>
              {entry.cloud_processing.status === "stale"
                ? "Review again"
                : cloudDispatchLabel(entry.cloud_processing.dispatch_status)}
            </span>
          </div>
          <p class="cloud-explanation">
            {entry.data_class.value === "c0"
              ? "Build a bounded copy and inspect it before a cloud task can use it."
              : "Redact obvious identifiers locally, then inspect the exact derivative before approval."}
          </p>
          <button
            class="btn cloud-preview-button"
            disabled={preparingCloudPreview || approvingCloudPreview}
            onclick={prepareCloudPreview}
          >
            {preparingCloudPreview
              ? "Preparing locally…"
              : entry.cloud_processing.status === "staged"
                ? "Review a fresh copy"
                : "Preview cloud-ready copy"}
          </button>
          {#if entry.cloud_processing.status === "staged"}
            {#if entry.cloud_processing.dispatch_status !== "not_queued"}
              <dl class="cloud-job">
                <div><dt>Provider role</dt><dd class="mono">{entry.cloud_processing.provider_role}</dd></div>
                <div><dt>Task</dt><dd>{entry.cloud_processing.task ?? "Content analysis"}</dd></div>
                <div><dt>State</dt><dd>{cloudDispatchLabel(entry.cloud_processing.dispatch_status)}</dd></div>
                <div><dt>Provider calls</dt><dd>{entry.cloud_processing.provider_calls}</dd></div>
              </dl>
              {#if entry.cloud_processing.dispatch_status === "queued"}
                <button
                  class="btn btn-primary cloud-run-button"
                  disabled={runningCloudJob}
                  onclick={runCloudJob}
                >
                  {runningCloudJob ? "Sending approved copy…" : "Run approved cloud analysis"}
                </button>
                <p class="cloud-boundary">This sends only the reviewed derivative to the selected provider.</p>
              {:else if entry.cloud_processing.dispatch_status === "failed"}
                {#if entry.cloud_processing.last_error}
                  <p class="cloud-job-error">{entry.cloud_processing.last_error}</p>
                {/if}
                <button
                  class="btn cloud-run-button"
                  disabled={runningCloudJob || entry.cloud_processing.provider_calls >= 5}
                  onclick={runCloudJob}
                >
                  {runningCloudJob ? "Retrying…" : "Retry approved cloud analysis"}
                </button>
                {#if entry.cloud_processing.provider_calls >= 5}
                  <p class="cloud-boundary">The five-call retry limit has been reached. Review and stage a fresh derivative.</p>
                {/if}
              {:else if entry.cloud_processing.dispatch_status === "running"}
                <p class="cloud-boundary">A provider call is in progress. Reload the entry to refresh its state.</p>
              {/if}

              {#if entry.cloud_processing.dispatch_status === "succeeded" && entry.cloud_processing.result}
                {@const result = entry.cloud_processing.result}
                <div class="cloud-result" aria-label="Cloud content analysis">
                  <div class="cloud-result-head">
                    <strong>{result.importance} importance</strong>
                    {#if entry.cloud_processing.completed_at}
                      <span>{lifecycleDate(entry.cloud_processing.completed_at)}</span>
                    {/if}
                  </div>
                  <p>{result.summary}</p>
                  <p class="cloud-rationale">{result.importance_rationale}</p>
                  {#if result.important_dates.length > 0}
                    <div class="cloud-result-group">
                      <span class="section-label">Important dates</span>
                      {#each result.important_dates as date, index (`${date.label}:${date.date}:${date.source_text}`)}
                        {@const candidate = calendarCandidates.find((item) => item.key === `important_dates:${index}`)}
                        <div class="cloud-result-item">
                          <strong>{date.label}</strong>
                          <span>{date.date ?? "Date not resolved"}</span>
                          <small>{date.source_text}</small>
                          {#if candidate}
                            <button
                              class="calendar-proposal-button"
                              disabled={calendarProposalBusy !== null || calendarProposalSaved.has(candidate.key)}
                              onclick={() => proposeToCalendar(candidate)}
                            >
                              {calendarProposalSaved.has(candidate.key)
                                ? "In Calendar review"
                                : calendarProposalBusy === candidate.key
                                  ? "Adding…"
                                  : "Add to Calendar review"}
                            </button>
                          {/if}
                        </div>
                      {/each}
                    </div>
                  {/if}
                  {#if result.action_items.length > 0}
                    <div class="cloud-result-group">
                      <span class="section-label">Actions</span>
                      {#each result.action_items as action, index (`${action.text}:${action.due_date}`)}
                        {@const candidate = calendarCandidates.find((item) => item.key === `action_items:${index}`)}
                        <div class="cloud-result-item">
                          <strong>{action.text}</strong>
                          {#if action.due_date}<span>Due {action.due_date}</span>{/if}
                          {#if candidate}
                            <button
                              class="calendar-proposal-button"
                              disabled={calendarProposalBusy !== null || calendarProposalSaved.has(candidate.key)}
                              onclick={() => proposeToCalendar(candidate)}
                            >
                              {calendarProposalSaved.has(candidate.key)
                                ? "In Calendar review"
                                : calendarProposalBusy === candidate.key
                                  ? "Adding…"
                                  : "Add deadline to Calendar review"}
                            </button>
                          {/if}
                        </div>
                      {/each}
                    </div>
                  {/if}
                  {#if result.topics.length > 0}
                    <div class="cloud-topics" aria-label="Topics">
                      {#each result.topics as topic (topic)}<span>{topic}</span>{/each}
                    </div>
                  {/if}
                  {#if calendarProposalSaved.size > 0}
                    <a class="calendar-review-link" href={link("/calendar")}>Open Calendar review →</a>
                  {/if}
                  {#if calendarProposalError}
                    <p class="cloud-job-error" role="alert">{calendarProposalError}</p>
                  {/if}
                </div>
              {/if}
            {/if}
          {/if}
          {#if entry.cloud_processing.dispatch_status === "not_queued"}
            <!-- The roster sits outside the `staged` branch on purpose. Which
                 providers exist, what data class each accepts and how much of
                 today's budget is left are facts about this machine, not about
                 this item's derivative. Gating them behind a Preview press made
                 a working cloud path look like an unbuilt one. -->
            {#if cloudProviders.length > 0}
              <div class="provider-roster" role="radiogroup" aria-labelledby="cloud-provider-roster-title">
                <p class="section-label" id="cloud-provider-roster-title">Cloud providers</p>
                {#each cloudProviders as provider (provider.role)}
                  <label
                    class="provider-option"
                    class:provider-selected={provider.role === selectedProviderRole}
                    class:provider-unavailable={!provider.available}
                  >
                    <span class="provider-head">
                      <input
                        type="radio"
                        name="cloud-provider"
                        value={provider.role}
                        bind:group={selectedProviderRole}
                        disabled={!provider.available || queueingCloudDerivative}
                      />
                      <strong>{provider.name}</strong>
                      <span class="mono">{provider.model}</span>
                    </span>
                    <dl class="provider-facts">
                      <div><dt>Role</dt><dd class="mono">{provider.role}</dd></div>
                      <div><dt>Endpoint</dt><dd>{provider.provider_label} · {provider.location}</dd></div>
                      <div><dt>Accepts</dt><dd>{providerTierLabel(provider.data_tier)}</dd></div>
                      <div><dt>Billing</dt><dd>{providerBillingLabel(provider)}</dd></div>
                      <div><dt>Failover</dt><dd>Priority {provider.failover_priority}</dd></div>
                      <div><dt>Budget today</dt><dd>{providerBudgetLabel(provider)}</dd></div>
                      <div><dt>Input ceiling</dt><dd>{provider.max_input_tokens.toLocaleString("en-GB")} tokens</dd></div>
                      <div>
                        <dt>State</dt>
                        <dd>
                          {providerStateLabel(provider)}
                          {#if provider.unavailable_reason}
                            <span class="mono">{provider.unavailable_reason}</span>
                          {/if}
                        </dd>
                      </div>
                    </dl>
                  </label>
                {/each}
              </div>
              <button
                class="btn cloud-queue-button"
                disabled={queueingCloudDerivative
                  || !selectedProviderRole
                  || entry.cloud_processing.status !== "staged"}
                onclick={queueCloudDerivative}
              >
                {queueingCloudDerivative ? "Queueing…" : "Queue for cloud processing"}
              </button>
              {#if entry.cloud_processing.status !== "staged"}
                <p class="provider-missing">Queueing needs a reviewed copy of this item first — preview one above.</p>
              {/if}
              {#if !cloudProviders.some((provider) => provider.available)}
                <p class="provider-missing">The reviewed providers are configured, but their keys have not been materialized on this machine.</p>
              {/if}
            {:else if providerListLoaded}
              <p class="provider-missing">No reviewed HTTPS cloud role is configured.</p>
            {/if}
            <p class="cloud-boundary">Approval stages only the reviewed copy. Queueing records provider intent without sending it.</p>
          {:else if entry.cloud_processing.dispatch_status === "queued"}
            <p class="cloud-boundary">Queued locally. Nothing has been sent until you run the approved analysis.</p>
          {/if}
          {#if cloudPreviewError && !cloudPreview}
            <p class="inline-error" aria-live="polite">{cloudPreviewError}</p>
          {/if}
        </section>
        {/if}

        {#if entry.mail}
          <section aria-labelledby="mail-classification-title">
            <p class="section-label" id="mail-classification-title">Mail classification</p>
            <dl>
              <div><dt>Category</dt><dd>{mailCategory}</dd></div>
              <div><dt>Method</dt><dd>{entry.mail.classification_method}</dd></div>
              <div><dt>Revision</dt><dd class="mono">{entry.mail.classification_version}</dd></div>
              {#if entry.mail.gmail_action_at}
                <div><dt>Last Sjel action</dt><dd>{entry.mail.gmail_action}</dd></div>
                <div><dt>Changed</dt><dd>{lifecycleDate(entry.mail.gmail_action_at)}</dd></div>
              {/if}
              {#if entry.mail.gmail_location}
                <div><dt>Gmail location</dt><dd>{entry.mail.gmail_location}</dd></div>
              {/if}
              {#if entry.mail.gmail_observed_at}
                <div><dt>Last checked</dt><dd>{lifecycleDate(entry.mail.gmail_observed_at)}</dd></div>
              {/if}
              {#if entry.mail.gmail_sync_status}
                <div><dt>Sync</dt><dd>{entry.mail.gmail_sync_status}</dd></div>
              {/if}
              {#if entry.mail.gmail_sync_action}
                <div><dt>Pending action</dt><dd>{entry.mail.gmail_sync_action}</dd></div>
              {/if}
              {#if entry.mail.purge_after}
                <div><dt>Sjel cleanup</dt><dd>{lifecycleDate(entry.mail.purge_after)}</dd></div>
              {/if}
            </dl>
            <p class="classification-rationale">{entry.mail.rationale}</p>
            {#if entry.mail.gmail_sync_error}
              <p class="inline-error">{entry.mail.gmail_sync_error}</p>
            {/if}
          </section>
        {/if}

        {#if entry.evaluation}
          <section aria-labelledby="evaluation-title">
            <p class="section-label" id="evaluation-title">Why is this here?</p>
            <EvaluationBreakdown evaluation={entry.evaluation} />
          </section>
        {/if}

        {#if entry.relevance.length > 0}
          <section aria-labelledby="relevance-title">
            <div class="aside-title">
              <p class="section-label" id="relevance-title">Matches</p>
              <span>{entry.relevance[0].mode}</span>
            </div>
            {#each entry.relevance as match (match.profile_key)}
              <div class="match">
                <div class="match-head">
                  <strong>{match.profile_label}</strong>
                  <span class="mono">{match.score.toFixed(2)}</span>
                </div>
                <details>
                  <summary>Classification</summary>
                  <p>{match.rationale}</p>
                </details>
              </div>
            {/each}
          </section>
        {/if}

        {#if entry.processing.length > 0}
          <section aria-labelledby="processing-title">
            <p class="section-label" id="processing-title">Processing provenance</p>
            <dl class="processing-ledger">
              {#each entry.processing as stage (stage.stage)}
                <div>
                  <dt>{STAGE_LABEL[stage.stage] ?? stage.stage}</dt>
                  <dd>
                    <span>{stage.tier}</span>
                    <span class="mono" use:tip={stage.revision}>{stage.revision}</span>
                  </dd>
                </div>
              {/each}
            </dl>
          </section>
        {/if}

        {#if entry.origins.length > 0}
          <section>
            <p class="section-label">Found via</p>
            {#each entry.origins as origin (`${origin.source_id}:${origin.source_ref}`)}
              <p class="origin">
                {origin.label ?? origin.source_id}
                <span class="mono">{origin.source_ref}</span>
              </p>
            {/each}
          </section>
        {/if}
      </aside>
    </div>
  </article>

  {#if cloudPreview}
    <div class="modal-backdrop">
      <div
        class="preview-modal"
        use:modal={{
          onClose: () => (cloudPreview = null),
          canClose: () => !approvingCloudPreview,
        }}
        role="dialog"
        aria-modal="true"
        aria-labelledby="cloud-preview-title"
        tabindex="-1"
      >
        <header class="preview-head">
          <div>
            <p class="section-label">Local preview</p>
            <h2 id="cloud-preview-title">Review the cloud-ready copy</h2>
          </div>
          <button
            class="icon-button"
            type="button"
            aria-label="Close preview"
            disabled={approvingCloudPreview}
            onclick={() => (cloudPreview = null)}
          >×</button>
        </header>

        <div class="preview-facts">
          <span>{dataClassLabel(cloudPreview.original_data_class)} source</span>
          <span>→</span>
          <span>{dataClassLabel(cloudPreview.derivative_data_class)} derivative</span>
          <span>{cloudPreview.redaction_count} redactions</span>
          <span>{cloudPreview.truncated ? "Bounded at 16,000 characters" : "Complete bounded document"}</span>
        </div>

        <!-- PRD Q9b: a reduced call says so in words, above the per-kind ledger.
             The count line in preview-facts is a number; this is the sentence,
             and a gate nobody can see is indistinguishable from one that is off. -->
        {#if cloudPreview.redaction_receipt}
          <p class="redaction-receipt">{cloudPreview.redaction_receipt}</p>
        {/if}

        {#if cloudPreview.redactions.length > 0}
          <div class="redaction-ledger" aria-label="Local entity redactions">
            <span class="section-label">Detected locally</span>
            {#each cloudPreview.redactions as finding (finding.entity_type)}
              <span><code>{finding.marker}</code> {finding.entity_type.replaceAll("_", " ")} × {finding.count}</span>
            {/each}
          </div>
        {/if}

        <div class="preview-document" aria-label="Document proposed for cloud processing">
          <pre>{cloudPreview.document}</pre>
        </div>

        <div class="preview-limitations">
          <p class="section-label">Review limits</p>
          <ul>
            {#each cloudPreview.limitations as limitation}
              <li>{limitation}</li>
            {/each}
          </ul>
        </div>

        {#if cloudPreviewError}<p class="inline-error" aria-live="polite">{cloudPreviewError}</p>{/if}
        <footer class="preview-actions">
          <p>This approval stores the exact reviewed copy locally. Provider calls: 0.</p>
          <button class="btn" disabled={approvingCloudPreview} onclick={() => (cloudPreview = null)}>Cancel</button>
          <button class="btn btn-primary" disabled={approvingCloudPreview} onclick={approveCloudPreview}>
            {approvingCloudPreview ? "Staging…" : "Approve cloud-ready copy"}
          </button>
        </footer>
      </div>
    </div>
  {/if}
{/if}

<style>
  .back {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    margin: 0 0 2rem;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    text-decoration: none;
  }

  article {
    width: 100%;
    max-width: 96rem;
    margin: 0 auto;
  }

  .article-head {
    margin: 0 0 2.5rem;
  }

  .overline,
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.55rem;
  }

  .overline {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .read-pill {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.1rem 0.4rem;
    border-radius: var(--radius-full);
    background: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    font-weight: 500;
  }

  h1 {
    max-width: 27ch;
    margin: 0.9rem 0 0;
    font-size: clamp(2rem, 3.25vw, 3.35rem);
    font-weight: 560;
    line-height: 1.04;
    letter-spacing: -0.04em;
  }

  .byline {
    margin: 0.65rem 0 0;
    color: var(--text-secondary);
  }

  /* Where a feed article carries a byline, a calendar item carries when it
     happens and how binding it is. Same slot, same weight. */
  .when {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
    margin: 0.65rem 0 0;
    color: var(--text-secondary);
    font-size: 0.875rem;
  }

  .when-fields {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.3rem;
  }

  .when-fields input[type="date"],
  .when-fields input[type="time"] {
    padding: 0.1rem 0.25rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-sm);
    font-variant-numeric: tabular-nums;
    font-weight: 600;
  }

  .when-fields input:hover:not(:disabled) { border-color: var(--card-border); }
  .when-fields input:focus { border-color: var(--primary); outline: none; }
  .when-fields input:disabled { opacity: 0.6; }
  .when-fields .dash { color: var(--text-tertiary); }

  .all-day {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .when-problem {
    margin: 0.35rem 0 0;
    color: var(--danger);
    font-size: var(--text-xs);
  }

  /* Not an error — a standing fact about this entry, so it reads as a note
     rather than something the operator did wrong. */
  .export-note {
    margin: 0.3rem 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .where-row {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    margin: 0.3rem 0 0;
    color: var(--text-tertiary);
  }

  /* Editable, but not shouting about it: a field reads as text until you touch
     it. A page full of visible input chrome would make reading the harder job,
     and reading is what this surface is for. */
  .title-edit {
    display: block;
    width: 100%;
    margin: 0;
    padding: 0;
    border: 0;
    border-bottom: 1.5px solid transparent;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: clamp(1.75rem, 3.5vw, 2.75rem);
    font-weight: 620;
    letter-spacing: -0.03em;
  }

  .where-edit {
    flex: 1;
    min-width: 8rem;
    padding: 0.1rem 0.2rem;
    border: 0;
    border-bottom: 1px solid transparent;
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: 0.875rem;
  }

  .notes-edit {
    display: block;
    width: 100%;
    margin-top: 0.5rem;
    padding: 0.5rem 0.6rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-primary);
    font: inherit;
    font-size: var(--text-base);
    line-height: 1.6;
    resize: vertical;
  }

  .title-edit:hover,
  .where-edit:hover,
  .notes-edit:hover {
    border-color: var(--card-border);
  }

  .title-edit:focus,
  .where-edit:focus,
  .notes-edit:focus {
    border-color: var(--primary);
    outline: none;
  }

  .title-edit:disabled,
  .where-edit:disabled,
  .notes-edit:disabled {
    opacity: 0.6;
  }

  .kind-edit {
    padding: 0.1rem 0.2rem;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: right;
  }

  .kind-edit:hover { border-color: var(--card-border); }
  .kind-edit:focus { border-color: var(--primary); outline: none; }

  .commitment-set {
    display: inline-flex;
    gap: 0.2rem;
  }

  /* The same vocabulary the month grid uses: how filled it is says how binding
     it is. A calendar item is never given a score, so this is its only rank. */
  .commitment {
    padding: 0.1rem 0.45rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: var(--text-2xs);
    font-weight: 600;
    cursor: pointer;
  }

  .commitment:hover:not(:disabled) {
    border-color: var(--primary);
    color: var(--primary);
  }

  .commitment:disabled { cursor: default; }

  .commitment-planned.chosen {
    border-color: var(--primary);
    color: var(--primary);
  }

  .commitment-committed.chosen {
    border-color: var(--primary);
    background: var(--primary);
    color: var(--card-bg);
  }

  .commitment-possible.chosen {
    border-color: var(--text-secondary);
    color: var(--text-primary);
  }

  .links {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin: 1rem 0 0;
    padding: 0;
    list-style: none;
  }

  .links a {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }

  .links a:hover {
    border-color: var(--primary);
    color: var(--primary);
  }

  .actions {
    margin-top: 1.25rem;
  }

  .kept {
    color: var(--success);
  }

  .danger {
    color: var(--error, #c33);
  }

  .missing-source {
    color: var(--text-tertiary);
    cursor: default;
    opacity: 0.75;
  }

  .mail-redaction-notice {
    margin: 0 0 0.5rem;
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }

  .mail-category {
    display: flex;
    align-items: center;
    gap: 0.45rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .mail-category select {
    min-height: 2rem;
    padding: 0.35rem 0.55rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
  }

  .data-class-control {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 0.75rem;
    margin-bottom: 0.55rem;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .data-class-control select {
    min-height: 2rem;
    padding: 0.35rem 0.55rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--card-bg);
    color: var(--text-primary);
    font: inherit;
  }

  .data-class-value {
    margin: 0 0 0.55rem;
    color: var(--text-primary);
    font-size: 0.9rem;
    font-weight: 560;
  }

  .gmail-confirm {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    max-width: 52rem;
    margin-top: 0.85rem;
    padding: 0.75rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--surface);
  }

  .gmail-confirm p {
    flex: 1;
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-xs);
  }

  .reader-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(18rem, 22rem);
    align-items: start;
    gap: clamp(2rem, 4vw, 4.5rem);
  }

  .section-label {
    margin: 0 0 0.8rem;
    color: var(--primary);
    font-family: var(--font-mono);
    font-size: 0.625rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
  }

  .reader {
    min-width: 0;
    max-width: 70ch;
  }

  .note {
    margin-bottom: 3rem;
    padding: 1.25rem 0 1.25rem 1.4rem;
    border-left: 2px solid var(--primary);
  }

  .digest-note,
  .digest-focus,
  .digest-redactions,
  .digest-provenance,
  .classification-rationale {
    margin: 0;
    color: var(--text-secondary);
    font-size: 0.8rem;
    line-height: 1.55;
  }

  .digest {
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .digest-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 1rem;
  }

  /* Which rung produced this, at a glance. Muted: it explains the digest, it
     is not a second thing to read. */
  .digest-rung {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    letter-spacing: 0.02em;
  }

  .digest-redactions {
    color: var(--warning-ink);
  }

  .digest-provenance {
    color: var(--text-tertiary);
    font-size: 0.7rem;
  }

  .digest-provenance code {
    font-family: var(--font-mono);
    font-size: 0.7rem;
    overflow-wrap: anywhere;
  }

  .digest-controls {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 0.75rem;
    padding-top: 0.25rem;
    border-top: 1px solid var(--card-border);
  }

  .digest-field {
    display: flex;
    flex: 1 1 16rem;
    flex-direction: column;
    gap: 0.3rem;
  }

  .digest-field span {
    color: var(--text-tertiary);
    font-size: 0.7rem;
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .digest-field input {
    padding: 0.45rem 0.6rem;
    border: 1px solid var(--input-border);
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text-primary);
    font-family: inherit;
    font-size: 0.85rem;
  }

  .digest-buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
  }

  .classification-rationale {
    margin-top: 0.65rem;
  }

  .policy-rationale {
    margin: 0.45rem 0 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    line-height: 1.5;
  }

  .cloud-explanation,
  .cloud-boundary {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.5;
  }

  .cloud-preview-button {
    width: 100%;
    justify-content: center;
    margin-top: 0.8rem;
  }

  .provider-roster {
    display: grid;
    gap: 0.5rem;
    margin-top: 0.85rem;
  }

  .provider-roster .section-label {
    margin: 0;
  }

  .provider-option {
    display: grid;
    gap: 0.3rem;
    padding: 0.55rem 0.6rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--card-bg);
    cursor: pointer;
  }

  .provider-selected {
    border-color: var(--primary);
  }

  .provider-unavailable {
    cursor: not-allowed;
    opacity: 0.72;
  }

  .provider-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.4rem;
    color: var(--text-primary);
    font-size: var(--text-sm);
  }

  .provider-head .mono {
    color: var(--text-tertiary);
    font-size: 0.625rem;
    overflow-wrap: anywhere;
  }

  .provider-facts div {
    gap: 0.6rem;
    padding: 0.12rem 0;
    font-size: 0.625rem;
  }

  .provider-facts dd {
    overflow-wrap: anywhere;
  }

  .provider-facts .mono {
    color: var(--text-tertiary);
  }

  .provider-missing {
    margin: 0.4rem 0 0;
    color: var(--text-tertiary);
    font-size: 0.625rem;
    line-height: 1.45;
    overflow-wrap: anywhere;
  }

  .cloud-queue-button,
  .cloud-run-button {
    width: 100%;
    justify-content: center;
    margin-top: 0.65rem;
  }

  .cloud-job {
    margin-top: 0.8rem;
    padding-top: 0.55rem;
    border-top: 1px solid var(--card-border);
  }

  .cloud-boundary {
    margin-top: 0.55rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .cloud-job-error {
    margin: 0.65rem 0 0;
    color: var(--warning-ink);
    font-size: var(--text-2xs);
    line-height: 1.45;
  }

  .cloud-result {
    display: grid;
    gap: 0.7rem;
    margin-top: 0.8rem;
    padding-top: 0.8rem;
    border-top: 1px solid var(--card-border);
  }

  .cloud-result p {
    margin: 0;
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.5;
  }

  .cloud-result .cloud-rationale {
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .cloud-result-head,
  .cloud-result-item {
    display: grid;
    gap: 0.15rem;
  }

  .calendar-proposal-button {
    justify-self: start;
    margin-top: 0.2rem;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--primary);
    cursor: pointer;
    font: inherit;
    font-size: 0.625rem;
  }

  .calendar-proposal-button:disabled {
    cursor: default;
    opacity: 0.65;
  }

  .calendar-review-link {
    color: var(--primary);
    font-size: var(--text-2xs);
  }

  .cloud-result-head {
    grid-template-columns: 1fr auto;
    align-items: baseline;
    font-size: var(--text-xs);
  }

  .cloud-result-head strong {
    text-transform: capitalize;
  }

  .cloud-result-head span,
  .cloud-result-item span,
  .cloud-result-item small {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .cloud-result-group {
    display: grid;
    gap: 0.45rem;
  }

  .cloud-result-item {
    padding-left: 0.6rem;
    border-left: 1px solid var(--card-border);
    font-size: var(--text-2xs);
  }

  .cloud-topics {
    display: flex;
    flex-wrap: wrap;
    gap: 0.3rem;
  }

  .cloud-topics span {
    padding: 0.2rem 0.4rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    color: var(--text-secondary);
    font-size: 0.625rem;
  }

  .staged {
    color: var(--success) !important;
  }

  .source-document {
    min-width: 0;
  }

  .transcript-disclosure {
    border-top: 1px solid var(--card-border);
    border-bottom: 1px solid var(--card-border);
  }

  .transcript-disclosure > summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 1rem;
    padding: 1.1rem 0;
    cursor: pointer;
    list-style: none;
  }

  .transcript-disclosure > summary::-webkit-details-marker {
    display: none;
  }

  .transcript-disclosure > summary::after {
    content: "+";
    flex: 0 0 auto;
    color: var(--primary);
    font-family: var(--font-mono);
    font-size: 1rem;
  }

  .transcript-disclosure[open] > summary::after {
    content: "−";
  }

  .transcript-disclosure .section-label {
    display: block;
    margin-bottom: 0.15rem;
  }

  .transcript-disclosure strong {
    font-size: 1rem;
    font-weight: 560;
  }

  .disclosure-meta {
    margin-left: auto;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .transcript-body {
    position: relative;
    padding: 1.25rem 0 2rem;
    border-top: 1px solid var(--card-border);
  }

  .source-link {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    margin-bottom: 1.25rem;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .document-head {
    display: flex;
    align-items: end;
    justify-content: space-between;
    gap: 1rem;
    margin-bottom: 1.4rem;
    padding-bottom: 0.75rem;
    border-bottom: 1px solid var(--card-border);
  }

  .document-head .section-label {
    margin-bottom: 0.15rem;
  }

  .document-head h2 {
    margin: 0;
    font-size: 1.25rem;
  }

  .document-head a {
    display: inline-flex;
    align-items: center;
    gap: 0.3rem;
    flex-shrink: 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .empty {
    padding: 1rem 0;
    color: var(--text-tertiary);
    font-size: 0.875rem;
  }

  .context {
    position: sticky;
    top: 6rem;
  }

  .context section {
    padding: 1.1rem 0;
    border-top: 1px solid var(--card-border);
  }

  .context section:first-child {
    padding-top: 0;
    border-top: 0;
  }

  dl {
    margin: 0;
  }

  dl div {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.25rem 0;
    font-size: var(--text-xs);
  }

  dt {
    color: var(--text-tertiary);
  }

  dd {
    margin: 0;
    color: var(--text-secondary);
    text-align: right;
  }

  .processing-ledger div {
    align-items: flex-start;
  }

  .processing-ledger dd {
    display: flex;
    min-width: 0;
    max-width: 12rem;
    flex-direction: column;
    gap: 0.08rem;
  }

  .processing-ledger .mono {
    overflow: hidden;
    color: var(--text-tertiary);
    font-size: 0.5625rem;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .aside-title,
  .match-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 0.75rem;
  }

  .aside-title > span {
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .match {
    padding: 0.7rem 0;
    border-top: 1px solid var(--card-border);
  }

  .match-head {
    font-size: var(--text-sm);
  }

  .match-head span {
    color: var(--primary);
  }

  .match details {
    margin-top: 0.25rem;
  }

  .match summary {
    cursor: pointer;
    color: var(--text-tertiary);
    font-size: 0.625rem;
  }

  .match p,
  .origin {
    margin: 0.35rem 0 0;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    line-height: 1.5;
  }

  .origin span {
    display: block;
    margin-top: 0.15rem;
    color: var(--text-tertiary);
  }

  .modal-backdrop {
    position: fixed;
    z-index: 100;
    inset: 0;
    display: grid;
    place-items: center;
    padding: 1.5rem;
    background: color-mix(in srgb, var(--background) 72%, transparent);
    backdrop-filter: blur(8px);
  }

  .preview-modal {
    display: flex;
    width: min(62rem, 100%);
    max-height: min(52rem, calc(100vh - 3rem));
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-lg);
    background: var(--background);
    box-shadow: 0 1.5rem 5rem rgb(0 0 0 / 0.18);
  }

  .preview-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 1rem;
    padding: 1.4rem 1.5rem 1rem;
  }

  .preview-head .section-label {
    margin-bottom: 0.35rem;
  }

  .preview-head h2 {
    margin: 0;
    font-size: 1.35rem;
    font-weight: 580;
    letter-spacing: -0.02em;
  }

  .icon-button {
    width: 2rem;
    height: 2rem;
    border: 0;
    background: transparent;
    color: var(--text-tertiary);
    font: inherit;
    font-size: 1.4rem;
    cursor: pointer;
  }

  .preview-facts {
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
    padding: 0 1.5rem 1rem;
  }

  .preview-facts span {
    padding: 0.3rem 0.5rem;
    border: 1px solid var(--card-border);
    border-radius: 999px;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .redaction-receipt {
    margin: 0.75rem 0 0;
    padding: 0.5rem 0.65rem;
    border-left: 3px solid var(--warning);
    background: var(--warning-soft);
    color: var(--text-primary);
    font-size: 0.78rem;
  }

  .redaction-ledger {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.4rem 0.75rem;
    padding: 0.7rem 1rem;
    border-bottom: 1px solid var(--card-border);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
  }

  .redaction-ledger .section-label {
    margin: 0;
  }

  .redaction-ledger code {
    color: var(--primary);
    font-family: var(--font-mono);
  }

  .preview-document {
    min-height: 12rem;
    flex: 1;
    overflow: auto;
    margin: 0 1.5rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
    background: var(--surface);
  }

  .preview-document pre {
    margin: 0;
    padding: 1.25rem;
    color: var(--text-primary);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: 1.6;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .preview-limitations {
    padding: 1rem 1.5rem 0;
  }

  .preview-limitations .section-label {
    margin-bottom: 0.4rem;
  }

  .preview-limitations ul {
    margin: 0;
    padding-left: 1rem;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    line-height: 1.5;
  }

  .preview-actions {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    padding: 1rem 1.5rem 1.35rem;
  }

  .preview-actions p {
    flex: 1;
    margin: 0;
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
  }

  .state {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    color: var(--text-secondary);
  }

  .error,
  .inline-error {
    color: var(--warning-ink);
  }

  .inline-error {
    font-size: var(--text-xs);
  }

  @media (max-width: 62rem) {
    .article-head {
      margin-bottom: 2.5rem;
    }

    .reader-grid {
      grid-template-columns: minmax(0, 48rem);
      justify-content: center;
    }

    .context {
      position: static;
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
      gap: 1.5rem;
      margin-top: 3rem;
      padding-top: 1rem;
      border-top: 1px solid var(--card-border);
    }

    .context section,
    .context section:first-child {
      padding: 0;
      border: 0;
    }
  }

  @media (max-width: 40rem) {
    .article-head {
      margin-bottom: 2rem;
    }

    h1 {
      font-size: 2.15rem;
    }

    .document-head {
      align-items: flex-start;
      flex-direction: column;
    }

    .gmail-confirm {
      align-items: stretch;
      flex-direction: column;
    }

    .transcript-disclosure > summary {
      align-items: flex-start;
    }

    .disclosure-meta {
      display: none;
    }

    .note {
      padding-left: 1rem;
    }

    .modal-backdrop {
      padding: 0;
    }

    .preview-modal {
      width: 100%;
      max-height: 100vh;
      min-height: 100vh;
      border: 0;
      border-radius: 0;
    }

    .preview-actions {
      align-items: stretch;
      flex-direction: column;
    }

    .preview-actions p {
      width: 100%;
    }
  }
</style>
