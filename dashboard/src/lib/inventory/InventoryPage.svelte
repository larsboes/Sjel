<script lang="ts">
  // The household's rows: what is owned at /inventory, what is wanted at /wishlist. One editor
  // and one offline-aware save path for both. The floor plans stay on /interior.
  import { onMount, tick } from "svelte";
  import Icon from "$lib/Icon.svelte";
  import PageHeader from "$lib/PageHeader.svelte";
  import { tip } from "$lib/tip";
  import { link } from "$lib/nav";
  import {
    sjelStatus,
    interior,
    inventory,
    InventoryConflict,
    type InventoryItem,
    type InventoryRow,
    type InventoryState,
    type InventoryWishlist,
    type InteriorImpact,
  } from "$lib/api";
  import { capabilities } from "$lib/capabilities.svelte";
  import { isRoomPlanPhone } from "$lib/roomplan";
  import { SYNC_CHANGED_EVENT } from "$lib/sync-status";
  import { bridgedSrc } from "$lib/bridged-url";
  import { getConnectionSettings } from "$lib/mac-bridge";
  import { modal } from "$lib/modal";
  import Wishlist from "./Wishlist.svelte";
  import { euro } from "./money";

  let { view }: { view: "inventory" | "wishlist" } = $props();

  let loading = $state(true);
  let error = $state<string | null>(null);
  let phoneOnly = $state(false);
  let starting = $state(false);
  let rows = $state<InventoryRow[]>([]);
  let wishlist = $state<InventoryWishlist | null>(null);

  const size = (i: InventoryItem): string =>
    i.width !== null && i.depth !== null ? `${i.width} × ${i.depth}${i.height !== null ? ` × ${i.height}` : ""} cm` : "—";

  /** The same measurement for a card's meta line, where an unknown size is left out rather
      than printed as a lone "—" on a line of its own. */
  const knownSize = (i: InventoryItem): string | null => (i.width !== null && i.depth !== null ? size(i) : null);

  /**
   * Phone width, by the phone breakpoint `app.css` names (38rem). Below it the item editor
   * is a bottom sheet over the list and a card's note is clamped; above it nothing changes.
   * Read from the browser rather than guessed, and followed when the window is resized.
   */
  let phone = $state(false);
  onMount(() => {
    const query = window.matchMedia("(width < 38rem)");
    const sync = () => (phone = query.matches);
    sync();
    query.addEventListener("change", sync);
    return () => query.removeEventListener("change", sync);
  });

  // A garment carries its own `category`; furniture predates the field and is told by `kind`.
  const groupOf = (i: InventoryItem): string =>
    i.category ?? (i.kind === "slot" ? "needs" : i.kind === "gear" ? "gear" : "furniture");
  let group = $state("all");
  const allOwned = $derived(rows.filter((r) => r.state === "owned"));
  const ownedGroups = $derived.by(() => {
    const counts = new Map<string, number>();
    for (const r of allOwned) counts.set(groupOf(r.item), (counts.get(groupOf(r.item)) ?? 0) + 1);
    return [...counts.entries()].sort(([a], [b]) => a.localeCompare(b));
  });
  const owned = $derived(group === "all" ? allOwned : allOwned.filter((r) => groupOf(r.item) === group));
  const gone = $derived(rows.filter((r) => r.state === "gone"));
  /** Owned id → label of the wanted item that names it in `replaces`. */
  const replacedBy = $derived(
    new Map(
      rows
        .filter((r) => r.state === "wanted")
        .flatMap((r) => (r.item.replaces ?? []).map((id) => [id, r.item.label] as const)),
    ),
  );

  let editing = $state<string | null>(null);
  let editorEl = $state<HTMLElement | null>(null);
  /** The editor's first field, focused when it opens. */
  let firstField = $state<HTMLInputElement | null>(null);
  /** The sheet's scrolling body on a phone, so a result can be brought into view inside it. */
  let sheetBody = $state<HTMLElement | null>(null);
  let impactEl = $state<HTMLElement | null>(null);

  /**
   * Focus back on the Edit button of item `id`, looked up rather than remembered. Safari does
   * not focus a button it was tapped on, so `document.activeElement` at open is `<body>`; and
   * a save reloads the list, which replaces the button that was pressed.
   */
  function focusEditButton(id: string): void {
    document.querySelector<HTMLElement>(`button[data-edit="${CSS.escape(id)}"]`)?.focus();
  }

  // The page behind a bottom sheet does not scroll under the reader's thumb.
  $effect(() => {
    if (!phone || editing === null) return;
    const root = document.documentElement;
    const before = root.style.overflow;
    root.style.overflow = "hidden";
    return () => {
      root.style.overflow = before;
    };
  });
  /** Form values are strings while they are being typed; `patchFrom` converts on the way out. */
  type Draft = {
    label: string;
    width: string | number;
    depth: string | number;
    height: string | number;
    price_cents: string | number;
    priority: string;
    notes: string;
    reason: string;
    opens: string;
    open_clear: string | number;
    wall_ok: string;
    access_sides: string | number;
    access_clear: string | number;
    room_divider: boolean;
    image: string;
    category: string;
    link: string;
    size: string;
    color: string;
    seasons: string;
  };
  let draft = $state<Draft>({
    label: "", width: "", depth: "", height: "", price_cents: "", priority: "", notes: "",
    reason: "", opens: "", open_clear: "", wall_ok: "", access_sides: "",
    access_clear: "", room_divider: false, image: "", category: "", link: "",
    size: "", color: "", seasons: "",
  });
  let impact = $state<InteriorImpact | null>(null);
  let impactBusy = $state(false);
  let saving = $state(false);
  let saveError = $state<string | null>(null);
  /** The revision the open form was read at; sent as If-Match so a newer save elsewhere is not overwritten. */
  let editingRevision = $state<number | undefined>(undefined);
  /** A save that lost the race (409): what this form tried to change, and the values that stand now. */
  let conflict = $state<{ patch: Record<string, unknown>; current: InventoryItem | null } | null>(null);
  let history = $state<{ state: InventoryState; since: string; note: string | null }[]>([]);
  let creating = $state(false);
  let draftNew = $state({
    id: "",
    label: "",
    kind: "piece" as "piece" | "slot",
    state: "wanted" as InventoryState,
    width: "",
    depth: "",
    height: "",
    price: "",
    category: "",
    link: "",
    size: "",
    color: "",
    seasons: "",
  });

  const byId = $derived(new Map(rows.map((r) => [r.item.id, r])));
  /** Categories come from the rows; a list written here would drift from them. */
  const kategorien = $derived(
    [...new Set(rows.map((r) => r.item.category).filter((c): c is string => !!c))].sort(),
  );
  /**
   * Items whose shown values include an edit that has not reached the Mac, and items whose
   * queued edit the Mac refused as stale. Both are set only in the app, by the device's
   * outbox (`src-tauri/src/sync.rs`); the web shell never sees them.
   */
  const pendingIds = $derived(new Set(rows.filter((r) => r.pending).map((r) => r.item.id)));
  const conflictIds = $derived(new Set(rows.filter((r) => r.conflict).map((r) => r.item.id)));

  /** Which placement fields a piece fills in, if any. Empty means the name heuristic still judges it. */
  const declares = (i: InventoryItem): string =>
    [
      i.opens !== null && i.opens !== undefined ? "opens" : null,
      i.open_clear !== null && i.open_clear !== undefined ? "open_clear" : null,
      i.expands_to !== null && i.expands_to !== undefined ? "expands" : null,
      i.access_sides !== null && i.access_sides !== undefined ? "access" : null,
      i.room_divider ? "room_divider" : null,
    ]
      .filter(Boolean)
      .join(" · ");

  /** cm and € come out of inputs as strings; empty means "clear it", not "zero". */
  const num = (v: unknown): number | null => {
    const t = String(v ?? "").trim();
    if (t === "") return null;
    const n = Number(t);
    return Number.isFinite(n) ? n : null;
  };

  async function openEditor(id: string): Promise<void> {
    const row = byId.get(id);
    if (!row) return;
    editing = id;
    saveError = null;
    conflict = null;
    impact = null;
    const i = row.item;
    editingRevision = i.revision;
    draft = {
      label: i.label,
      width: i.width ?? "",
      depth: i.depth ?? "",
      height: i.height ?? "",
      price_cents: i.price_cents ?? "",
      priority: i.priority ?? "",
      notes: i.notes ?? "",
      reason: i.reason ?? "",
      opens: i.opens ?? "",
      open_clear: i.open_clear ?? "",
      wall_ok: i.wall_ok === null ? "" : String(i.wall_ok),
      access_sides: i.access_sides ?? "",
      access_clear: i.access_clear ?? "",
      room_divider: i.room_divider ?? false,
      image: i.image ?? "",
      category: i.category ?? "",
      link: i.link ?? "",
      size: i.size ?? "",
      color: i.color ?? "",
      // A list in the row, one comma-separated field in the form. The row keeps the list.
      // `?? []` because the app can talk to an interior build older than this field.
      seasons: (i.seasons ?? []).join(", "),
    };
    history = [];
    await tick();
    if (phone) {
      // A bottom sheet over the list: nothing to scroll to. `use:modal` has focused the
      // sheet; the first field is where the reader starts.
      firstField?.focus();
    } else if (editorEl) {
      // The editor renders below the whole list; without the scroll, Edit looks dead.
      firstField?.focus({ preventScroll: true });
      const margin = Number.parseFloat(getComputedStyle(editorEl).scrollMarginTop) || 0;
      window.scrollTo({
        top: editorEl.getBoundingClientRect().top + window.scrollY - margin,
        behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth",
      });
    }
    try {
      history = await inventory.stateHistory(id);
    } catch {
      // A missing history is not a reason to refuse the edit; the panel says so itself.
    }
  }

  /** Close the editor and hand focus back to the Edit button that opened it. */
  async function closeEditor(): Promise<void> {
    const id = editing;
    editing = null;
    impact = null;
    await tick();
    if (id !== null) focusEditButton(id);
  }

  /** On a phone, bring a part of the sheet into view inside the sheet's own scroll. */
  async function showInSheet(target: "impact" | "top"): Promise<void> {
    if (!phone) return;
    // After the tick, because the impact block is not rendered until `impact` is set.
    await tick();
    if (!sheetBody) return;
    const top = target === "top" ? 0 : impactEl ? impactEl.offsetTop - 8 : null;
    if (top !== null) sheetBody.scrollTo({ top, behavior: "smooth" });
  }

  /** Only the fields that actually differ. Sending the unchanged ones would work, but a diff
      that names exactly what moved is what makes the impact preview readable. */
  function patchFrom(id: string): Record<string, unknown> {
    const i = byId.get(id)?.item;
    if (!i) return {};
    const out: Record<string, unknown> = {};
    const put = (k: string, v: unknown, was: unknown) => {
      if (v !== was) out[k] = v;
    };
    put("label", String(draft.label ?? "").trim(), i.label);
    put("width", num(draft.width), i.width);
    put("depth", num(draft.depth), i.depth);
    put("height", num(draft.height), i.height);
    put("price_cents", num(draft.price_cents), i.price_cents);
    put("priority", String(draft.priority ?? "").trim() || null, i.priority);
    put("notes", String(draft.notes ?? "").trim() || null, i.notes);
    put("reason", String(draft.reason ?? "").trim() || null, i.reason);
    put("opens", String(draft.opens ?? "") || null, i.opens);
    put("open_clear", num(draft.open_clear), i.open_clear);
    put("wall_ok", draft.wall_ok === "" ? null : draft.wall_ok === "true", i.wall_ok);
    put("access_sides", num(draft.access_sides), i.access_sides);
    put("access_clear", num(draft.access_clear), i.access_clear);
    put("room_divider", draft.room_divider ? true : null, i.room_divider);
    put("image", String(draft.image ?? "").trim() || null, i.image);
    put("category", String(draft.category ?? "").trim() || null, i.category);
    put("link", String(draft.link ?? "").trim() || null, i.link);
    put("size", String(draft.size ?? "").trim() || null, i.size);
    put("color", String(draft.color ?? "").trim() || null, i.color);
    put(
      "seasons",
      String(draft.seasons ?? "")
        .split(",")
        .map((s) => s.trim())
        .filter(Boolean),
      i.seasons,
    );
    return out;
  }

  async function preview(): Promise<void> {
    if (editing === null) return;
    const patch = patchFrom(editing);
    if (Object.keys(patch).length === 0) {
      impact = null;
      return;
    }
    impactBusy = true;
    saveError = null;
    try {
      impact = await interior.impact(editing, patch);
      void showInSheet("impact");
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
      void showInSheet("top");
    } finally {
      impactBusy = false;
    }
  }

  async function save(): Promise<void> {
    if (editing === null) return;
    const patch = patchFrom(editing);
    if (Object.keys(patch).length === 0) {
      void closeEditor();
      return;
    }
    saving = true;
    saveError = null;
    conflict = null;
    try {
      const id = editing;
      await inventory.patchItem(id, patch, editingRevision);
      editing = null;
      impact = null;
      await load();
      await tick();
      focusEditButton(id);
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
      void showInSheet("top");
      if (caught instanceof InventoryConflict) {
        // Show what stands now and keep the form open. Nothing is retried: saving again is a
        // deliberate overwrite, measured against the fresh values and their revision.
        conflict = { patch, current: caught.current?.item ?? null };
        const fresh = caught.current;
        if (fresh) {
          rows = rows.map((r) => (r.item.id === fresh.item.id ? fresh : r));
          editingRevision = fresh.item.revision;
        }
      }
    } finally {
      saving = false;
    }
  }

  /** One field from the wishlist's side peek. A conflict reloads, so the peek shows what stands now. */
  async function patchWish(item: InventoryItem, patch: Partial<InventoryItem>): Promise<void> {
    try {
      await inventory.patchItem(item.id, patch, item.revision);
    } finally {
      // Not `load()`: its loading flag unmounts the list and closes the peek mid-edit.
      [rows, wishlist] = await Promise.all([inventory.rows(), inventory.wishlist()]);
    }
  }

  async function move(id: string, to: InventoryState): Promise<void> {
    saving = true;
    saveError = null;
    try {
      await inventory.setState(id, to);
      if (editing === id) history = await inventory.stateHistory(id);
      await load();
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      saving = false;
    }
  }

  async function create(): Promise<void> {
    saving = true;
    saveError = null;
    try {
      await inventory.createItem({
        id: draftNew.id.trim(),
        kind: draftNew.kind,
        label: draftNew.label.trim(),
        state: draftNew.state,
        width: num(draftNew.width),
        depth: num(draftNew.depth),
        height: num(draftNew.height),
        price_cents: num(draftNew.price),
        // Blank means blank: `""` would store an empty string that looks like a value.
        category: draftNew.category.trim() || null,
        link: draftNew.link.trim() || null,
        size: draftNew.size.trim() || null,
        color: draftNew.color.trim() || null,
        seasons: draftNew.seasons
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean),
      });
      creating = false;
      draftNew = {
        id: "",
        label: "",
        kind: "piece",
        state: "wanted",
        width: "",
        depth: "",
        height: "",
        price: "",
        category: "",
        link: "",
        size: "",
        color: "",
        seasons: "",
      };
      await load();
    } catch (caught) {
      saveError = caught instanceof Error ? caught.message : String(caught);
    } finally {
      saving = false;
    }
  }


  async function load(): Promise<void> {
    loading = true;
    error = null;
    try {
      [rows, wishlist] = await Promise.all([inventory.rows(), inventory.wishlist()]);
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      loading = false;
    }
  }

  async function start(): Promise<void> {
    starting = true;
    error = null;
    try {
      await sjelStatus.start("interior");
      await capabilities.refresh();
      await load();
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught);
    } finally {
      starting = false;
    }
  }

  // The outbox sent or dropped an edit: show the canonical node's answer instead of the pending values.
  onMount(() => {
    const reload = () => {
      if (rows.length === 0) return;
      void inventory.rows().then((r) => (rows = r)).catch(() => {});
      void inventory.wishlist().then((w) => (wishlist = w)).catch(() => {});
    };
    window.addEventListener(SYNC_CHANGED_EVENT, reload);
    return () => window.removeEventListener(SYNC_CHANGED_EVENT, reload);
  });


  onMount(() => {
    if (isRoomPlanPhone()) {
      // On the phone the rows come from the canonical node through the native bridge.
      void getConnectionSettings()
        .then((settings) => settings.canonical_base_url)
        .catch(() => null)
        .then((baseUrl) => {
          if (baseUrl) return load();
          phoneOnly = true;
          loading = false;
        });
      return;
    }
    capabilities.subscribe();
    void load();
  });
</script>

{#if view === "wishlist"}
  <PageHeader badge="Household" title="Wishlist" desc="What you still want to buy, and what it costs together." />
{:else}
  <PageHeader
    badge="Household"
    title="Inventory"
    desc="Everything you own or need, from furniture to the clothes a packing list draws on."
  />
{/if}

{#if phoneOnly}
  <div class="card offer">
    <p class="lead">Set the Sjel connection (page footer) to load the inventory from the canonical node.</p>
  </div>
{:else if error}
  <div class="card offer">
    <p class="lead"><Icon name="alert" size={15} /> {error}</p>
    <button onclick={start} disabled={starting}>
      {starting ? "Starting…" : "Start interior"}
    </button>
  </div>
{:else if loading}
  <p class="empty">Reading the inventory…</p>
{/if}

{#if !loading && !error && !phoneOnly}
  {#if view === "inventory" && wishlist}
    <p class="note"><a href={link("/wishlist")}>{wishlist.items.length} wanted items are on the wishlist</a></p>
  {/if}
  {#if view === "wishlist" && wishlist}
    <section class="card budget">
      <h2>What is still missing</h2>
      <p class="sum">
        <strong>{euro(wishlist.total_low_cents)}</strong>
        {#if wishlist.total_high_cents > wishlist.total_low_cents}
          <span class="upper">to {euro(wishlist.total_high_cents)}</span>
        {/if}
        <span class="over">across {wishlist.items.length} open items</span>
      </p>
      {#if wishlist.unpriced_count > 0}
        <p class="caveat">
          <Icon name="alert" size={14} />
          {wishlist.unpriced_count} of them carry no price at all and count as zero — the
          total is a floor, not an estimate.
        </p>
      {/if}
      {#if wishlist.monthly_balance}
        <p class="finance">
          Finance measures a median monthly balance of
          <strong class:negative={wishlist.monthly_balance.median_cents < 0}>
            {euro(wishlist.monthly_balance.median_cents)}
          </strong>
          over {wishlist.monthly_balance.months} complete months
          ({wishlist.monthly_balance.from} – {wishlist.monthly_balance.to}).
          {#if wishlist.months_to_pay !== null}
            That is <strong>{wishlist.months_to_pay.toFixed(1)} months</strong> of surplus.
          {:else}
            Nothing accrues out of a balance that is not positive, so there is no number of
            months to give.
          {/if}
        </p>
      {:else}
        <p class="caveat">Finance has written nothing on this machine, so there is no balance to measure against.</p>
      {/if}
    </section>

    <h3 class="group">Wanted <span class="count">{wishlist.items.length}</span></h3>
    <Wishlist
      items={wishlist.items}
      pending={pendingIds}
      conflicts={conflictIds}
      busy={saving}
      onPatch={patchWish}
      onBought={(id) => move(id, "owned")}
      onFullEditor={openEditor}
    />
  {/if}

  {#if view === "inventory"}
  <h3 class="group">Owned <span class="count">{owned.length}</span></h3>
  <div class="group-chips" role="group" aria-label="Filter owned by group">
    <button type="button" class:active={group === "all"} onclick={() => (group = "all")}>
      All <span class="count">{allOwned.length}</span>
    </button>
    {#each ownedGroups as [g, n] (g)}
      <button type="button" class:active={group === g} onclick={() => (group = g)}>
        {g} <span class="count">{n}</span>
      </button>
    {/each}
  </div>
  <ul class="items">
    {#each owned as { item: i } (i.id)}
      <li class="card item" class:has-shot={!!i.image}>
        <div class="head">
          <span class="label">{i.label}</span>
          {#if i.on_move}<span class="tag">{i.on_move}</span>{/if}
          {#if replacedBy.has(i.id)}<span class="tag" use:tip={`On the wishlist: ${replacedBy.get(i.id)}`}>replacement wanted</span>{/if}
          {#if pendingIds.has(i.id)}<span class="tag pending" use:tip={"Saved on this device; not on the canonical node yet."}>not synced</span>{/if}
          {#if conflictIds.has(i.id)}<span class="tag conflict" use:tip={"The canonical node changed this item too. Review it in the sync line at the top."}>conflict</span>{/if}
        </div>
        {#if i.image}
          <img class="shot" use:bridgedSrc={interior.mediaUrl(i.image)} alt={i.label} loading="lazy" />
        {/if}
        {#if knownSize(i) || i.estimated.length > 0 || i.measured_at}
          <p class="meta">
            {#if knownSize(i)}<span class="dims mono">{knownSize(i)}</span>{/if}
            {#if i.estimated.length > 0}
              <span class="guess" use:tip={"measured? no."}>~ {i.estimated.join(", ")}</span>
            {:else if i.measured_at}
              <span class="measured mono">measured {i.measured_at.slice(0, 10)}</span>
            {/if}
          </p>
        {/if}
        {#if declares(i)}
          <span class="declares mono" use:tip={"This piece states what it needs, so the name heuristic does not run for it."}>
            declares {declares(i)}
          </span>
        {/if}
        {#if i.category || i.color || i.size}
          <p class="meta">{[i.category, i.color, i.size].filter(Boolean).join(" · ")}</p>
        {/if}
        <div class="actions">
          <button class="ghost" data-edit={i.id} onclick={() => openEditor(i.id)}>Edit</button>
          {#if i.link}
            <a class="ghost" href={i.link} target="_blank" rel="noreferrer">Product page</a>
          {/if}
          <button class="ghost" disabled={saving} onclick={() => move(i.id, "gone")}>
            Given away
          </button>
        </div>
      </li>
    {/each}
  </ul>
  {/if}

  <!--
    One editor, two frames. On a phone it is a bottom sheet over the list, with Save and
    Cancel pinned under the fields; above the phone breakpoint it stays the card below the
    inventory it has always been. The fields, the impact preview and the history are the
    same snippets in both.
  -->
  {#snippet editorFields()}
    {#if saveError}<p class="err">{saveError}</p>{/if}
    {#if conflict}
      <div class="err conflict">
        <p>Saved on another device meanwhile. Nothing was written. Values now:</p>
        <ul>
          {#each Object.entries(conflict.patch) as [key, mine]}
            <li>
              <span class="mono">{key}</span>: now
              <strong>{JSON.stringify((conflict.current as Record<string, unknown> | null)?.[key] ?? null)}</strong>,
              yours {JSON.stringify(mine)}
            </li>
          {/each}
        </ul>
        <p>Save again to overwrite with yours.</p>
      </div>
    {/if}

    <div class="grid">
      <label class="f-full">label <input bind:this={firstField} bind:value={draft.label} /></label>
      <label class="f-third">width cm <input bind:value={draft.width} inputmode="numeric" /></label>
      <label class="f-third">depth cm <input bind:value={draft.depth} inputmode="numeric" /></label>
      <label class="f-third">height cm <input bind:value={draft.height} inputmode="numeric" /></label>
      <label class="f-half">price ct <input bind:value={draft.price_cents} inputmode="numeric" /></label>
      <label class="f-half">priority <input bind:value={draft.priority} /></label>
      <label class="f-half">category <input bind:value={draft.category} placeholder="kleidung" list="interior-categories" /></label>
      <label class="f-half">size <input bind:value={draft.size} placeholder="M" /></label>
      <label class="f-half">colour <input bind:value={draft.color} placeholder="weiss" /></label>
      <label class="f-half">season <input bind:value={draft.seasons} placeholder="winter, uebergang" /></label>
      <label class="f-full">link <input bind:value={draft.link} placeholder="https://…" inputmode="url" /></label>
    </div>

    <label class="wide">
      picture <input bind:value={draft.image} placeholder="produkt/…-01-….png" />
    </label>
    <p class="note">
      A path below the private interior asset root. Kept out of the bundle and fetched only
      when shown.
    </p>
    {#if draft.image}
      <img class="shot big" use:bridgedSrc={interior.mediaUrl(String(draft.image))} alt="" />
    {/if}
    <label class="wide">note <textarea rows="2" bind:value={draft.notes}></textarea></label>
    <label class="wide">reasoning <textarea rows="2" bind:value={draft.reason}></textarea></label>

    <h5>What this piece needs</h5>
    <p class="note">
      Fill any of these and the name heuristic stops judging this piece — it is checked against
      exactly what it states and nothing else. Leave them empty and it is still
      measured by its name.
    </p>
    <div class="grid">
      <label class="f-half">
        opens
        <select bind:value={draft.opens}>
          <option value="">— not stated —</option>
          <option value="north">north</option>
          <option value="south">south</option>
          <option value="east">east</option>
          <option value="west">west</option>
        </select>
      </label>
      <label class="f-half">open_clear cm <input bind:value={draft.open_clear} inputmode="numeric" /></label>
      <label>
        wall_ok
        <select bind:value={draft.wall_ok}>
          <option value="">— not stated —</option>
          <option value="true">true — may sit against a wall</option>
          <option value="false">false — that side is useless there</option>
        </select>
      </label>
      <label class="f-half">access_sides <input bind:value={draft.access_sides} inputmode="numeric" /></label>
      <label class="f-half">access_clear cm <input bind:value={draft.access_clear} inputmode="numeric" /></label>
      <label class="check">
        <input type="checkbox" bind:checked={draft.room_divider} />
        raumtrenner — meant to stand free
      </label>
    </div>

  {/snippet}

  {#snippet editorAfter()}
    <!--
      The reason this is a button and not a surprise. Declaring which side the wardrobe opens
      costs 2 or 4 layouts depending on the direction, and nothing said so until it was worked
      out by hand. The capability re-checks every layout here; the page only prints the answer.
    -->
    {#if impact}
      <div class="impact" bind:this={impactEl} class:worse={impact.passing_after < impact.passing_before}>
        <strong>
          {impact.passing_before} → {impact.passing_after} of {impact.layouts} layouts pass
        </strong>
        {#if impact.changed.length === 0}
          <p>No verdict moves. Safe to save.</p>
        {:else}
          <ul>
            {#each impact.changed as g (g.layout)}
              <li>
                <span class="mono">{g.layout}</span>
                {g.before.pass ? "passes" : "fails"} → {g.after.pass ? "passes" : "fails"}
                {#if g.after.hard.length > 0}
                  <span class="mono rules">{g.after.hard.join(", ")}</span>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>
    {/if}

    <h5>State history</h5>
    {#if history.length === 0}
      <p class="note">No history recorded.</p>
    {:else}
      <ul class="history">
        {#each history as h, n (h.since + n)}
          <li>
            <span class="mono st">{h.state}</span>
            <span class="mono when">{h.since.slice(0, 10)}</span>
            {#if h.note}<span class="why">{h.note}</span>{/if}
          </li>
        {/each}
      </ul>
      <p class="note">
        Appended, never overwritten — a wish that gets bought is a second row, and that span is
        what the wishlist joins to money with.
      </p>
    {/if}
  {/snippet}

  {#if editing !== null}
    {@const row = byId.get(editing)}
    {#if phone}
      <div class="sheet-wrap">
        <button class="sheet-backdrop" aria-label="Close editor" onclick={() => !saving && closeEditor()}></button>
        <div
          class="sheet editor"
          bind:this={editorEl}
          use:modal={{ onClose: closeEditor, canClose: () => !saving }}
          role="dialog"
          aria-modal="true"
          aria-labelledby="item-editor-title"
          tabindex="-1"
        >
          <div class="sheet-grip" aria-hidden="true"></div>
          <div class="sheet-head">
            <h4 id="item-editor-title">{row?.item.label ?? editing} <span class="mono id">{editing}</span></h4>
            <button type="button" class="btn btn-soft" onclick={closeEditor}>Close</button>
          </div>
          <div class="sheet-body" bind:this={sheetBody}>
            {@render editorFields()}
            {@render editorAfter()}
          </div>
          <div class="actions sheet-foot">
            <button class="ghost" disabled={impactBusy} onclick={preview}>
              {impactBusy ? "Checking…" : "Check impact"}
            </button>
            <button class="ghost" disabled={saving} onclick={closeEditor}>Cancel</button>
            <button disabled={saving} onclick={save}>{saving ? "Saving…" : "Save"}</button>
          </div>
        </div>
      </div>
    {:else}
      <div class="card editor" bind:this={editorEl}>
        <h4>{row?.item.label ?? editing} <span class="mono id">{editing}</span></h4>

        {@render editorFields()}

        <div class="actions">
          <button class="ghost" disabled={impactBusy} onclick={preview}>
            {impactBusy ? "Checking…" : "Check impact"}
          </button>
          <button disabled={saving} onclick={save}>{saving ? "Saving…" : "Save"}</button>
          <button class="ghost" onclick={closeEditor}>Close</button>
        </div>

        {@render editorAfter()}
      </div>
    {/if}
  {/if}

  <div class="addrow">
    <button class="ghost" onclick={() => (creating = !creating)}>
      {creating ? "Cancel" : "+ New entry"}
    </button>
  </div>

  {#if creating}
    <form class="card editor" onsubmit={(e) => { e.preventDefault(); create(); }}>
      <h4>New entry</h4>
      <div class="grid">
        <label>id <input bind:value={draftNew.id} placeholder="stehlampe" required /></label>
        <label>label <input bind:value={draftNew.label} placeholder="IKEA …" required /></label>
        <label>
          kind
          <select bind:value={draftNew.kind}>
            <option value="piece">piece — a thing, or a specific product</option>
            <option value="slot">slot — a need with target sizes, no product yet</option>
          </select>
        </label>
        <label>
          state
          <select bind:value={draftNew.state}>
            <option value="wanted">wanted</option>
            <option value="owned">owned</option>
          </select>
        </label>
        <label class="f-third">width cm <input bind:value={draftNew.width} inputmode="numeric" /></label>
        <label class="f-third">depth cm <input bind:value={draftNew.depth} inputmode="numeric" /></label>
        <label class="f-third">height cm <input bind:value={draftNew.height} inputmode="numeric" /></label>
        <label>price ct <input bind:value={draftNew.price} inputmode="numeric" /></label>
        <label>
          category
          <input bind:value={draftNew.category} placeholder="kleidung" list="interior-categories" />
        </label>
        <datalist id="interior-categories">
          {#each kategorien as c}<option value={c}></option>{/each}
        </datalist>
        <label class="f-third">size <input bind:value={draftNew.size} placeholder="M" /></label>
        <label class="f-third">colour <input bind:value={draftNew.color} placeholder="weiss" /></label>
        <label class="f-third">season <input bind:value={draftNew.seasons} placeholder="winter, uebergang" /></label>
        <label class="f-full">
          link
          <input bind:value={draftNew.link} placeholder="https://…" inputmode="url" />
        </label>
      </div>
      <p class="note">
        State is required, not a default — an entry with no state joins to nothing and would
        never appear in any list. <code>interior wunsch &lt;url&gt;</code> fills the same row
        from a shared link: title from the page, price only if the page declares one.
      </p>
      <div class="actions">
        <button type="submit" disabled={saving || !draftNew.id.trim() || !draftNew.label.trim()}>
          {saving ? "Creating…" : "Create"}
        </button>
      </div>
    </form>
  {/if}

  {#if view === "inventory" && gone.length > 0}
    <h3 class="group">Gone <span class="count">{gone.length}</span></h3>
    <ul class="items">
      {#each gone as { item: i } (i.id)}
        <li class="card item muted">
          <div class="head"><span class="label">{i.label}</span></div>
          {#if knownSize(i)}<p class="meta"><span class="dims mono">{knownSize(i)}</span></p>{/if}
        </li>
      {/each}
    </ul>
  {/if}
{/if}

<style>
  .group-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 0.35rem;
    margin: 0 0 0.75rem;
  }
  .group-chips button {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: 999px;
    color: var(--text-secondary);
    cursor: pointer;
    font-size: var(--text-xs, 0.8rem);
    padding: 0.25rem 0.65rem;
    text-transform: capitalize;
  }
  .group-chips button.active {
    border-color: var(--primary);
    color: var(--text-primary);
  }
  .count {
    color: var(--text-tertiary);
    margin-left: 0.35rem;
  }


  .measured {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }


  /* Which plan, in which room. Above the drawing, because that is where the eye starts. */

  /* The list of plans and the way to add one are one grid column, not two. */


  /* The measurements of one piece, while the pointer is on it. */






  /* The rule's own wording, under the violation it explains. Quiet: the reader came for what
     is wrong, and stays for why it counts as wrong. */

  /* ─── Dragging ────────────────────────────────────────────────────────── */


  /* No text selection while a piece is being moved. */








  /* ─── Editing ─────────────────────────────────────────────────────────── */

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
    margin-top: 0.6rem;
  }

  .actions button {
    background: var(--primary);
    border: 1px solid transparent;
    border-radius: 6px;
    color: var(--card-bg);
    cursor: pointer;
    font: inherit;
    font-size: var(--text-xs);
    padding: 0.3rem 0.7rem;
  }

  .actions button.ghost {
    background: transparent;
    border-color: var(--card-border);
    color: var(--text-secondary);
  }

  .actions button:disabled {
    cursor: not-allowed;
    opacity: 0.5;
  }

  .addrow {
    margin: 1rem 0;
  }

  .addrow button {
    background: transparent;
    border: 1px dashed var(--card-border);
    border-radius: 6px;
    color: var(--text-secondary);
    cursor: pointer;
    font: inherit;
    font-size: 0.8rem;
    padding: 0.45rem 0.9rem;
  }

  .editor {
    margin: 1rem 0 1.5rem;
    padding: 1rem;
    /* Clears the sticky header and the sync line when scrolled to. */
    scroll-margin-top: calc(env(safe-area-inset-top, 0px) + 8rem);
  }

  .editor h4 {
    align-items: baseline;
    display: flex;
    gap: 0.5rem;
    margin: 0 0 0.75rem;
  }

  .editor h5 {
    margin: 1.1rem 0 0.3rem;
  }

  .editor .id {
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    font-weight: 400;
  }

  .editor .grid {
    display: grid;
    gap: 0.6rem;
    grid-template-columns: repeat(auto-fit, minmax(11rem, 1fr));
  }

  .editor label {
    color: var(--text-tertiary);
    display: flex;
    flex-direction: column;
    font-size: 0.7rem;
    gap: 0.2rem;
    text-transform: uppercase;
  }

  .editor label.wide {
    margin-top: 0.6rem;
  }

  .editor label.check {
    align-items: center;
    flex-direction: row;
    text-transform: none;
  }

  .editor input,
  .editor select,
  .editor textarea {
    background: var(--input-bg);
    border: 1px solid var(--card-border);
    border-radius: 5px;
    color: var(--text-primary);
    font: inherit;
    font-size: 0.8rem;
    padding: 0.35rem 0.45rem;
    text-transform: none;
    width: 100%;
  }

  .editor .note {
    color: var(--text-tertiary);
    font-size: 0.72rem;
    margin: 0.35rem 0 0;
  }

  .editor .err {
    color: var(--danger, #f87171);
    font-size: 0.78rem;
    margin: 0 0 0.6rem;
  }

  /* The arithmetic that used to have to be done by hand, one direction at a time. */
  .impact {
    background: var(--input-bg);
    border: 1px solid var(--card-border);
    border-left: 3px solid var(--primary);
    border-radius: 6px;
    font-size: 0.78rem;
    margin-top: 0.85rem;
    padding: 0.6rem 0.75rem;
  }

  .impact.worse {
    border-left-color: var(--warning);
  }

  .impact ul,
  .history {
    list-style: none;
    margin: 0.4rem 0 0;
    padding: 0;
  }

  .impact li,
  .history li {
    display: flex;
    flex-wrap: wrap;
    gap: 0.45rem;
    padding: 0.15rem 0;
  }

  .impact .rules {
    color: var(--warning-ink);
  }

  .history {
    font-size: 0.78rem;
  }

  .history .st {
    color: var(--primary);
    min-width: 4.5rem;
  }

  .history .when {
    color: var(--text-tertiary);
  }

  .history .why {
    color: var(--text-secondary);
  }

  .shot {
    border: 1px solid var(--card-border);
    border-radius: 6px;
    display: block;
    margin: 0.4rem 0;
    max-height: 8rem;
    object-fit: cover;
    width: 100%;
  }

  .shot.big {
    max-height: 18rem;
    object-fit: contain;
  }

  .declares {
    color: var(--primary);
    font-size: 0.7rem;
  }

  .caveat {
    align-items: center;
    color: var(--warning-ink);
    display: flex;
    font-size: var(--text-xs);
    gap: 0.35rem;
    margin: 0.85rem 0 0;
  }

  .budget h2 {
    font-size: var(--text-base);
    margin: 0 0 0.5rem;
  }
  .budget .sum {
    align-items: baseline;
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    margin: 0;
  }
  .budget .sum strong {
    color: var(--text-primary);
    font-size: 1.5rem;
  }
  .upper,
  .over {
    color: var(--text-tertiary);
    font-size: var(--text-sm);
  }
  .finance {
    color: var(--text-secondary);
    font-size: var(--text-sm);
    margin: 0.75rem 0 0;
  }
  .finance strong {
    color: var(--success);
  }
  .finance strong.negative {
    color: var(--danger);
  }

  .group {
    align-items: baseline;
    display: flex;
    font-size: var(--text-xs);
    gap: 0.35rem;
    letter-spacing: 0.06em;
    margin: 1.5rem 0 0.6rem;
    text-transform: uppercase;
  }

  .items {
    display: grid;
    gap: 0.5rem;
    grid-template-columns: repeat(auto-fill, minmax(16rem, 1fr));
    list-style: none;
    margin: 0;
    padding: 0;
  }
  /* A column, not a grid: a grid spreads a tall row's spare height between the lines. */
  .item {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    padding: 0.7rem 0.85rem;
  }
  .item > .actions {
    margin-top: auto;
    padding-top: 0.6rem;
  }
  .item.muted {
    opacity: 0.55;
  }
  .item .head {
    align-items: baseline;
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .item .label {
    color: var(--text-primary);
    font-size: 0.875rem;
  }
  .tag {
    background: var(--primary-soft);
    border-radius: var(--radius-sm);
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    padding: 0.05rem 0.35rem;
  }
  .tag.pending {
    background: var(--warning-soft);
    color: var(--text-primary);
  }
  .tag.conflict {
    background: var(--danger-soft, var(--warning-soft));
    color: var(--text-primary);
  }
  .dims {
    color: var(--text-secondary);
    font-size: var(--text-sm);
  }
  /* Size, price and the guess marker on one line. A part that is not known is left out. */
  .meta {
    align-items: baseline;
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
    margin: 0;
  }
  .guess {
    color: var(--warning-ink);
    font-size: var(--text-xs);
  }

  .card {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-radius: var(--radius-md);
  }
  .offer,
  .budget {
    padding: 1rem 1.15rem;
  }
  .offer .lead {
    align-items: center;
    color: var(--danger);
    display: flex;
    gap: 0.4rem;
    margin: 0 0 0.75rem;
  }
  .offer button {
    background: var(--primary);
    border: none;
    border-radius: var(--radius-sm);
    color: white;
    cursor: pointer;
    font-size: var(--text-sm);
    padding: 0.45rem 0.9rem;
  }
  .empty {
    color: var(--text-tertiary);
    font-size: var(--text-sm);
  }
  .mono {
    font-family: var(--font-mono);
  }

  /* By how much, not whether. */



  /* One row per day, one column per hour: a year on four lines. */




  /* ─── The editor as a bottom sheet (rendered only below the phone breakpoint) ─── */

  /* Above the header (50) and the bottom tab bar (45): a modal covers the shell. */
  .sheet-wrap {
    align-items: flex-end;
    display: flex;
    inset: 0;
    position: fixed;
    z-index: 100;
  }
  .sheet-backdrop {
    background: rgb(0 0 0 / 48%);
    border: 0;
    height: 100%;
    inset: 0;
    position: absolute;
    width: 100%;
  }
  .editor.sheet {
    background: var(--card-bg);
    border: 1px solid var(--card-border);
    border-bottom: 0;
    border-radius: var(--radius-xl) var(--radius-xl) 0 0;
    box-shadow: var(--glass-shadow);
    display: flex;
    flex-direction: column;
    margin: 0;
    max-height: 90dvh;
    padding: 0;
    position: relative;
    width: 100%;
  }
  .editor.sheet:focus,
  .editor.sheet:focus-visible {
    box-shadow: var(--glass-shadow);
    outline: none;
  }
  .sheet-grip {
    background: var(--rule);
    border-radius: 999px;
    flex: none;
    height: 0.3rem;
    margin: var(--space-3) auto 0;
    width: 2.5rem;
  }
  .sheet-head {
    align-items: center;
    border-bottom: 1px solid var(--card-border);
    display: flex;
    flex: none;
    gap: var(--space-3);
    justify-content: space-between;
    padding: var(--space-2) var(--space-5) var(--space-3);
  }
  .editor .sheet-head h4 {
    flex-wrap: wrap;
    font-size: var(--text-md);
    margin: 0;
    min-width: 0;
  }
  .sheet-head .btn {
    flex: none;
    min-height: 2.75rem;
  }
  /* The one part that scrolls; `position` makes it the offset parent `showInSheet` measures from. */
  .sheet-body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: var(--space-4) var(--space-5) var(--space-5);
    position: relative;
  }
  .actions.sheet-foot {
    background: var(--card-bg);
    border-top: 1px solid var(--card-border);
    flex: none;
    flex-wrap: nowrap;
    gap: var(--space-3);
    margin: 0;
    padding: var(--space-3) var(--space-5) calc(var(--space-3) + env(safe-area-inset-bottom, 0px));
  }
  .sheet-foot button {
    flex: 1 1 auto;
    font-size: var(--text-sm);
    min-height: 2.75rem;
    padding: 0 var(--space-3);
    white-space: nowrap;
  }

  /* ─── Phone width (the 38rem breakpoint `app.css` names) ─────────────────── */

  @media (width < 38rem) {
    /* One segmented line, pinned under the header. The header is its bar plus the top
       safe area plus a 1px rule; the band runs edge to edge over main's padding. */
    /* Opaque, not glass: labels over moving text were unreadable wherever the blur was
       missing, and a strip this thin gains nothing from it. */

    /* Denser cards: a photo becomes a thumbnail beside the text, the actions a short row. */
    .item {
      align-content: start;
      display: grid;
      gap: var(--space-2);
      padding: var(--space-4);
    }
    .item.has-shot {
      column-gap: var(--space-4);
      grid-template-columns: minmax(0, 1fr) 4.5rem;
    }
    .item.has-shot > * {
      grid-column: 1;
    }
    .item.has-shot > .shot {
      grid-column: 2;
      grid-row: 1 / span 2;
      height: 4.5rem;
      margin: 0;
      width: 4.5rem;
    }
    .item.has-shot > .actions {
      grid-column: 1 / -1;
    }
    .item > .actions {
      flex-wrap: nowrap;
      margin-top: var(--space-1);
      padding-top: 0;
    }
    .item .actions button {
      font-size: var(--text-sm);
      min-height: 2.5rem;
      padding: 0 var(--space-4);
    }

    .group {
      margin: var(--space-5) 0 var(--space-3);
    }

    /* Solve and Buy & declare: their cards had no inset and a page-sized heading. */
    section.card {
      padding: var(--space-4);
    }
    section.card h2 {
      font-size: var(--text-md);
      line-height: var(--leading-tight);
      margin: 0 0 var(--space-2);
    }
    /* A wide table scrolls inside its card instead of pushing the page sideways. */

        /* Every number and sentence stays; only the air between them goes. */
    .budget {
      padding: var(--space-4);
    }
    .budget h2 {
      font-size: var(--text-sm);
      margin: 0 0 var(--space-1);
    }
    .budget .sum {
      gap: var(--space-1) var(--space-3);
    }
    .budget .sum strong {
      font-size: var(--text-lg);
    }
    .budget .caveat {
      margin-top: var(--space-3);
    }
    /* A warning that wraps keeps its icon on the first line. */
    .caveat {
      align-items: flex-start;
    }
    .finance {
      font-size: var(--text-xs);
      line-height: var(--leading-normal);
      margin-top: var(--space-3);
    }

    /* Fields in rows that match what they measure: the three dimensions together, price
       beside priority. 16px or more, or iOS zooms the page on focus; 44px tall to tap. */
    .editor .grid {
      gap: var(--space-4) var(--space-3);
      grid-template-columns: repeat(6, minmax(0, 1fr));
    }
    .editor .grid > label {
      grid-column: span 6;
    }
    .editor .grid > label.f-half {
      grid-column: span 3;
    }
    .editor .grid > label.f-third {
      grid-column: span 2;
    }
    .editor label {
      font-size: var(--text-2xs);
    }
    .editor input,
    .editor select,
    .editor textarea {
      font-size: var(--text-base);
      min-height: 2.75rem;
      padding: var(--space-2) var(--space-3);
    }
    .editor input[type="checkbox"] {
      min-height: 0;
      width: auto;
    }
    .editor .note {
      font-size: var(--text-xs);
    }
  }
</style>
