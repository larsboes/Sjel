<script lang="ts">
  import { onMount } from "svelte";
  import { checkFareAlerts } from "$lib/travel/fare-alerts";
  import { base } from "$app/paths";
  import { page } from "$app/state";
  import "../app.css";
  import Icon from "$lib/Icon.svelte";
  import { capabilities } from "$lib/capabilities.svelte";
  import { PRIMARY_NAV, UTILITY_NAV, capabilityForPath, withoutCapabilities, link } from "$lib/nav";
  import { createShellStarter } from "$lib/shell-start";
  import { axonStatus } from "$lib/api";
  import SoundscapeDock from "$lib/SoundscapeDock.svelte";
  import AssistantDrawer from "$lib/assistant/AssistantDrawer.svelte";
  import { assistantStore } from "$lib/assistant/assistant.svelte";
  import MacConnection from "$lib/MacConnection.svelte";
  import DeviceRegistry from "$lib/DeviceRegistry.svelte";
  import { canClaimOnThisDevice } from "$lib/devices";
  import SetupSjel from "$lib/setup/SetupSjel.svelte";
  import JoinPrompt from "$lib/setup/JoinPrompt.svelte";
  import { inTauri } from "$lib/mac-bridge";
  import SyncStatus from "$lib/SyncStatus.svelte";
  import OmniSearch from "$lib/omni/OmniSearch.svelte";
  import { omniStore } from "$lib/omni/omni.svelte";

  let { children, data } = $props();

  let dark = $state(true);
  let menuOpen = $state(false);
  let moreOpen = $state(false);
  let now = $state(new Date());
  let moreEl: HTMLDetailsElement | undefined = $state();
  let drawerEl: HTMLElement | undefined = $state();

  /**
   * Capabilities this shell has already tried to start, for the lifetime of the tab.
   *
   * The guard itself lives in `$lib/shell-start` so that the test can run the real thing:
   * a set defined here and a copy of it in the test file left the regression it names
   * unfalsifiable. See shell-start.ts for why the set is written before the POST and never
   * cleared.
   */
  const claimStarts = createShellStarter();

  /**
   * Bumped once, when a start POST has actually brought a capability up.
   *
   * Starting the capability is not enough on its own: the page below has already run its
   * own `onMount` fetch against a service that was still down, and it holds the error
   * string it got. Re-keying the page subtree on this counter renders it again against the
   * capability that is now running. It never changes on a machine where everything is
   * already up, and `attemptedStarts` caps it at one bump per capability per tab.
   */
  let startedCount = $state(0);

  // `data.demo` is null outside a demo build, so both lists below are the untouched arrays
  // and the banner never renders. In a demo build the index names the capabilities the
  // recording could not include; the destinations that lead only to those are dropped
  // rather than left to render a page of error cards (#168).
  const demo = $derived(data?.demo ?? null);
  const missing = $derived(new Set(Object.keys(demo?.absent ?? {})));
  // Pointing at a map route is enough intent to start its ~1 MB library and the vendored
  // style. Fire-and-forget, idempotent, and a miss costs a download the user did not make —
  // which is why it hangs off hover and focus rather than off page load.
  let mapWarmed = false;
  function warmMap(item: { warmsMap?: true }): void {
    if (!item.warmsMap || mapWarmed) return;
    mapWarmed = true;
    void import("$lib/map/surface").then((surface) => surface.warm());
  }

  const primary = $derived(withoutCapabilities(PRIMARY_NAV, missing));
  const utility = $derived(withoutCapabilities(UTILITY_NAV, missing));

  const isActive = (href: string) =>
    href === "/" ? page.url.pathname === "/" : page.url.pathname.startsWith(href);
  const utilityActive = $derived(utility.some((item) => isActive(item.href)));

  onMount(() => {
    dark = window.matchMedia("(prefers-color-scheme: dark)").matches;
    const clock = setInterval(() => (now = new Date()), 1000);
    const stop = capabilities.subscribe();
    // A new Sparpreis low becomes a phone notification when the app opens or comes back.
    void checkFareAlerts();
    const onVisible = () => {
      if (document.visibilityState === "visible") void checkFareAlerts();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      clearInterval(clock);
      stop();
      document.removeEventListener("visibilitychange", onVisible);
    };
  });

  $effect(() => {
    document.documentElement.classList.toggle("dark", dark);
  });

  /**
   * Start what this page needs, in the shell, once per capability.
   *
   * /finance and /map were the only two primary destinations that started nothing, so on a
   * machine where those capabilities were stopped both rendered an error card and the
   * operator had to visit /capabilities. Home already solved this per kind; this is the
   * same fix one level up, so a page added tomorrow inherits it.
   *
   * The ONLY tracked read is `page.url.pathname`. Everything after the first `await` is
   * untracked, which is what keeps the fifteen-second capability poll from re-running this.
   */
  $effect(() => {
    const pathname = page.url.pathname;
    // A demo build has no start route: /map ships in the published nav with no fixture,
    // and posting to it answers 501 and logs an error a visitor cannot act on.
    if (data?.demo) return;
    const wanted = capabilityForPath(pathname);
    if (wanted.length === 0) return;

    void (async () => {
      await capabilities.refresh();
      const worthStarting = (name: string): boolean => {
        const capability = capabilities.byName(name);
        return Boolean(capability) && capability?.up !== true;
      };
      for (const name of claimStarts(wanted, worthStarting)) {
        const started = await axonStatus.start(name).then(
          (result) => result.up === true,
          // Swallowed: the page itself reports what it could not read, and a failed start
          // is not a second thing to tell the reader about.
          () => false,
        );
        if (started) startedCount += 1;
      }
      await capabilities.refresh();
    })();
  });

  function closeMore(event: MouseEvent): void {
    if (moreOpen && moreEl && !moreEl.contains(event.target as Node)) moreOpen = false;
  }

  function handleShellKeydown(event: KeyboardEvent): void {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      omniStore.toggle();
      return;
    }
    if (event.key === "/" && !["INPUT", "TEXTAREA"].includes((event.target as HTMLElement)?.tagName)) {
      event.preventDefault();
      omniStore.open();
      return;
    }
    if (event.key === "Escape") {
      moreOpen = false;
      menuOpen = false;
      return;
    }
    // A drawer that lets Tab walk out behind its own scrim is a drawer to a mouse only.
    if (event.key !== "Tab" || !menuOpen || !drawerEl) return;
    const stops = [...drawerEl.querySelectorAll<HTMLElement>("a[href], button:not([disabled])")];
    if (stops.length === 0) return;
    const first = stops[0];
    const last = stops[stops.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }
</script>

<svelte:window onkeydown={handleShellKeydown} onclick={closeMore} />

<div class="shell">
  <!-- Every page is one Tab away from its own content, rather than fourteen nav links. -->
  <a class="skip" href="#main">Skip to content</a>
  <header>
    <div class="bar">
      <div class="brand-group">
        <a class="brand" href={link("/")}>
          <span class="mark">S</span>
          <span class="name">Sjel</span>
        </a>

        <button
          type="button"
          class="omni-trigger"
          onclick={() => omniStore.open()}
          aria-label="Search across Sjel"
        >
          <Icon name="search" size={13} />
          <span class="omni-trigger-text">Search life...</span>
          <kbd class="omni-trigger-kbd">⌘K</kbd>
        </button>
      </div>

      <div class="meta">
        <button
          type="button"
          class="btn ask-btn"
          onclick={() => { assistantStore.restoreFloating(); assistantStore.toggle(); }}
          aria-label="Ask Sjel Assistant"
          title="Ask Sjel Assistant"
        >
          <Icon name="sparkles" size={14} />
          <span class="ask-btn-text">Ask</span>
        </button>
        <span class="clock mono">
          <Icon name="clock" size={14} />
          {now.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" })}
        </span>
        <button class="btn" onclick={() => (dark = !dark)} aria-label="Switch theme">
          <Icon name={dark ? "sun" : "moon"} />
        </button>
        <button
          class="btn burger"
          onclick={() => (menuOpen = !menuOpen)}
          aria-label="Menu"
          aria-expanded={menuOpen}
        >
          <Icon name={menuOpen ? "close" : "menu"} />
        </button>
      </div>
    </div>

    <div class="desktop-wrap">
      <nav class="desktop" aria-label="Main navigation">
        {#each primary as item (item.href)}
          <a
            class="nav-link"
            class:active={isActive(item.href)}
            href={link(item.href)}
            aria-current={isActive(item.href) ? "page" : undefined}
            onmouseenter={() => warmMap(item)}
            onfocus={() => warmMap(item)}
          >
            <Icon name={item.icon as never} size={14} />
            {item.label}
          </a>
        {/each}
      </nav>

      <details class="more" bind:this={moreEl} bind:open={moreOpen}>
        <summary class="nav-link" class:active={utilityActive}>
          <Icon name="boxes" size={14} />
          More
        </summary>
        <nav class="more-menu" aria-label="Projects and system">
          {#each utility as item (item.href)}
            <a
              class="nav-link"
              class:active={isActive(item.href)}
              href={link(item.href)}
              aria-current={isActive(item.href) ? "page" : undefined}
              onclick={() => (moreOpen = false)}
            >
              <Icon name={item.icon as never} size={14} />
              {item.label}
            </a>
          {/each}
        </nav>
      </details>
    </div>
  </header>

  <SyncStatus />

  {#if demo}
    <!-- Sticky under the header rather than dismissible: a visitor who scrolls past a
         one-time notice and then reads a balance is exactly who this is for. -->
    <aside class="demo-banner">
      <strong>{demo.label}</strong>
      <span>
        Every figure below was generated from seed <code>{demo.seed}</code> and dated around
        {demo.anchor}. Writing is disabled.
      </span>
      {#if missing.size > 0}
        <span class="demo-absent">Not in this demo: {[...missing].join(", ")}</span>
      {/if}
      <!-- The generated reference is a sibling of this bundle, not a route in it, so it is a
           plain href rather than link(): the router must not try to handle it. -->
      <a class="demo-docs" href="{base}/docs/index.html">Reference →</a>
    </aside>
  {/if}

  {#if menuOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="scrim" onclick={() => (menuOpen = false)}></div>
    <nav class="mobile" bind:this={drawerEl} aria-label="Mobile navigation">
      <span class="nav-section">Work</span>
      {#each primary as item (item.href)}
        <a
          class="nav-link"
          class:active={isActive(item.href)}
          href={link(item.href)}
          aria-current={isActive(item.href) ? "page" : undefined}
          onmouseenter={() => warmMap(item)}
          onfocus={() => warmMap(item)}
          onclick={() => (menuOpen = false)}
        >
          <Icon name={item.icon as never} />
          {item.label}
        </a>
      {/each}
      <span class="nav-section second">Projects and system</span>
      {#each utility as item (item.href)}
        <a
          class="nav-link"
          class:active={isActive(item.href)}
          href={link(item.href)}
          aria-current={isActive(item.href) ? "page" : undefined}
          onmouseenter={() => warmMap(item)}
          onfocus={() => warmMap(item)}
          onclick={() => (menuOpen = false)}
        >
          <Icon name={item.icon as never} />
          {item.label}
        </a>
      {/each}
    </nav>
  {/if}

  <main id="main">
    {#key startedCount}
      {@render children()}
    {/key}
  </main>

  {#if canClaimOnThisDevice()}
    <SetupSjel />
  {:else if !inTauri() && capabilities.byName("devices")}
    <JoinPrompt />
  {/if}

  <footer>
    <div class="inner">
      <span>Sjel</span>
      <span class="mono">{capabilities.items.length} capabilities</span>
      <MacConnection />
      <!-- An unregistered phone gets 401 for the capability list, so it would never see the
           link it needs to register (2026-09-27). The iOS app always shows it. -->
      {#if capabilities.byName("devices") || canClaimOnThisDevice()}
        <DeviceRegistry />
      {/if}
    </div>
  </footer>

  <nav class="mobile-tab-bar" aria-label="Mobile navigation">
    <a
      class="tab-link"
      class:active={isActive("/")}
      href={link("/")}
      aria-current={isActive("/") ? "page" : undefined}
    >
      <Icon name="home" size={18} />
      <span class="tab-label">Home</span>
    </a>
    <a
      class="tab-link"
      class:active={isActive("/calendar")}
      href={link("/calendar")}
      aria-current={isActive("/calendar") ? "page" : undefined}
    >
      <Icon name="calendar" size={18} />
      <span class="tab-label">Calendar</span>
    </a>
    <button
      type="button"
      class="tab-link tab-action"
      onclick={() => { assistantStore.restoreFloating(); assistantStore.toggle(); }}
      aria-label="Ask Sjel Assistant"
    >
      <span class="tab-action-icon">
        <Icon name="sparkles" size={18} />
      </span>
      <span class="tab-label">Ask</span>
    </button>
    <a
      class="tab-link"
      class:active={isActive("/feed")}
      href={link("/feed")}
      aria-current={isActive("/feed") ? "page" : undefined}
    >
      <Icon name="feed" size={18} />
      <span class="tab-label">Feed</span>
    </a>
    <button
      type="button"
      class="tab-link"
      class:active={menuOpen || utilityActive}
      onclick={() => (menuOpen = !menuOpen)}
      aria-label="More navigation options"
      aria-expanded={menuOpen}
    >
      <Icon name="menu" size={18} />
      <span class="tab-label">More</span>
    </button>
  </nav>

  <SoundscapeDock />
  <AssistantDrawer />
  <OmniSearch />
</div>

<style>
  .shell {
    min-height: 100vh;
    display: flex;
    flex-direction: column;
    /* Set by SoundscapeDock while it is mounted, absent otherwise. */
    padding-bottom: var(--soundscape-dock-height, 0px);
  }

  /* The one surface the whole page passes under, so it is the one that most has to read
   * as glass. Was `blur(12px)` with no saturation and no prefix: Safari is the browser
   * this is read in and needed the prefix, and blur without saturate desaturates whatever
   * scrolls behind it into fog. */
  header {
    position: sticky;
    top: 0;
    z-index: 50;
    padding-top: env(safe-area-inset-top, 0px);
    background-color: var(--header-bg);
    -webkit-backdrop-filter: var(--glass-blur);
    backdrop-filter: var(--glass-blur);
    border-bottom: 1px solid var(--header-border);
  }

  /* Translucent only while there is something behind it to see. At the top of the page
     the bar sits on flat background and the blur has nothing to do, so it goes opaque and
     loses its border — the rule appears as the page starts moving under it. */
  @supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
    header {
      background-color: var(--card-bg);
    }
  }

  /* `min(100%, …)` rather than a bare cap: below --shell-max the shell IS the viewport,
   * so the layout runs to the edges (minus each band's own padding) instead of sitting in
   * a column with gutters. The cap only takes over on a display wide enough that
   * full-bleed would stretch a line of text past reading comfort. See app.css for why
   * this is px and the type scale is not. */
  .bar,
  .desktop-wrap,
  main,
  footer .inner {
    max-width: min(100%, var(--shell-max));
    margin-inline: auto;
    width: 100%;
  }

  .bar {
    height: var(--header-h);
    padding-inline: var(--space-6);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    font-weight: 600;
    letter-spacing: -0.01em;
  }

  .brand-group {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    flex: 1;
    max-width: 32rem;
  }

  .omni-trigger {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.65rem;
    border-radius: var(--radius-md);
    background-color: var(--surface);
    border: 1px solid var(--card-border);
    color: var(--text-tertiary);
    font-size: var(--text-xs);
    cursor: pointer;
    transition: border-color 0.15s ease, color 0.15s ease, background-color 0.15s ease;
    max-width: 17rem;
    width: 100%;
  }

  .omni-trigger:hover {
    border-color: var(--primary);
    color: var(--text-secondary);
    background-color: var(--card-bg);
  }

  .omni-trigger-text {
    flex: 1;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .omni-trigger-kbd {
    font-size: var(--text-2xs);
    font-family: inherit;
    padding: 0.05rem 0.3rem;
    border-radius: var(--radius-sm);
    background-color: var(--card-bg);
    border: 1px solid var(--card-border);
    color: var(--text-tertiary);
  }

  .ask-btn {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.35rem 0.6rem;
    color: var(--primary);
    background-color: var(--primary-soft);
    border-color: transparent;
  }

  .ask-btn:hover {
    background-color: var(--primary);
    color: var(--text-inverse);
  }

  .ask-btn-text {
    font-size: var(--text-xs);
    font-weight: 600;
  }

  .mark {
    display: grid;
    place-items: center;
    height: 1.75rem;
    width: 1.75rem;
    border-radius: var(--radius-md);
    background-color: var(--primary);
    color: var(--text-inverse);
    font-size: 0.65rem;
    font-weight: 700;
  }

  .meta {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    color: var(--text-tertiary);
    font-size: var(--text-xs);
  }

  .clock {
    display: none;
    align-items: center;
    gap: 0.4rem;
    font-variant-numeric: tabular-nums;
  }

  .desktop-wrap {
    display: none;
    align-items: center;
    justify-content: space-between;
    padding: 0 1.5rem 0.25rem;
  }

  nav.desktop {
    display: flex;
    gap: 0.125rem;
  }

  .nav-link {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 0.75rem;
    font-size: var(--text-xs);
    font-weight: 500;
    border-radius: var(--radius-md);
    color: var(--nav-inactive);
    transition:
      color 0.15s ease,
      background-color 0.15s ease;
  }

  .nav-link:hover {
    color: var(--nav-hover);
    background-color: var(--primary-soft);
  }

  .nav-link.active {
    color: var(--primary);
    background-color: var(--primary-soft);
  }

  .more {
    position: relative;
  }

  .more summary {
    list-style: none;
    cursor: pointer;
  }

  .more summary::-webkit-details-marker {
    display: none;
  }

  .more-menu {
    position: absolute;
    top: calc(100% + 0.4rem);
    right: 0;
    width: 13rem;
    padding: 0.4rem;
    border: 1px solid var(--card-border);
    border-radius: var(--radius);
    background: var(--card-bg);
    box-shadow: var(--card-shadow-hover);
  }

  .more-menu .nav-link {
    width: 100%;
  }

  .demo-banner {
    position: sticky;
    top: var(--header-h);
    z-index: 45;
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 0.25rem 0.75rem;
    padding: 0.5rem clamp(1rem, 2.5vw, 1.5rem);
    border-bottom: 1px solid var(--card-border);
    /* colour-mix against the page rather than a new token: the banner has to read as a
       notice in both themes without app.css growing a palette entry only one build uses. */
    background: color-mix(in oklab, var(--primary-soft) 70%, var(--page-bg));
    color: var(--text-secondary);
    font-size: var(--text-xs);
    line-height: 1.5;
  }

  .demo-banner strong {
    color: var(--primary);
  }

  .demo-absent {
    color: var(--text-tertiary);
  }

  .demo-banner .demo-docs {
    margin-left: auto;
    color: var(--primary);
    white-space: nowrap;
  }

  .scrim {
    position: fixed;
    inset: var(--header-h) 0 0;
    z-index: 40;
    background-color: rgb(0 0 0 / 40%);
  }

  nav.mobile {
    position: fixed;
    inset: var(--header-h) 0 auto auto;
    z-index: 41;
    height: calc(100vh - var(--header-h));
    width: 16rem;
    padding: 1rem;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
    background-color: var(--page-bg);
    border-left: 1px solid var(--header-border);
  }

  nav.mobile .nav-link {
    padding: 0.75rem 1rem;
    font-size: 0.875rem;
  }

  /* Sentence case. A tracked-out all-caps label over each half of the drawer is template
     chrome; these are two short headings, and they read as headings without it. */
  .nav-section {
    padding: var(--space-2) var(--space-5) var(--space-1);
    color: var(--text-tertiary);
    font-size: var(--text-2xs);
    font-weight: 600;
  }

  .nav-section.second {
    margin-top: 0.75rem;
    padding-top: 1rem;
    border-top: 1px solid var(--card-border);
  }

  /* Denser by a step at the top end: 2rem rather than 2.5rem of gutter on a wide display,
     which is where the old value read as an airy admin template. */
  main {
    flex-grow: 1;
    padding: clamp(var(--space-5), 1.6vw, var(--space-7));
  }

  .skip {
    position: absolute;
    left: var(--space-3);
    top: calc(var(--header-h) * -2);
    z-index: 60;
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background: var(--card-bg);
    color: var(--primary);
    font-size: var(--text-sm);
    font-weight: 600;
    transition: top var(--motion-fast) ease;
  }

  .skip:focus-visible {
    top: var(--space-3);
  }

  footer {
    border-top: 1px solid var(--header-border);
    padding: 1rem 1.5rem;
    font-size: var(--text-xs);
    color: var(--text-tertiary);
  }

  footer .inner {
    display: flex;
    justify-content: space-between;
  }

  @media (width >= 48rem) {
    .clock,
    .desktop-wrap {
      display: flex;
    }

    .burger {
      display: none;
    }
  }

  .mobile-tab-bar {
    display: none;
  }

  /* The bar's own height comes from --header-h, which app.css redeclares as 3.25rem
     inside this same breakpoint. All six offsets above follow it without being restated. */
  @media (width < 38rem) {
    .shell {
      padding-bottom: calc(var(--soundscape-dock-height, 0px) + 3.75rem + env(safe-area-inset-bottom, 0px));
    }

    .mobile-tab-bar {
      display: flex;
      position: fixed;
      left: 0;
      right: 0;
      bottom: var(--soundscape-dock-height, 0px);
      z-index: 45;
      height: calc(3.5rem + env(safe-area-inset-bottom, 0px));
      padding-bottom: env(safe-area-inset-bottom, 0px);
      background-color: var(--header-bg);
      -webkit-backdrop-filter: var(--glass-blur);
      backdrop-filter: var(--glass-blur);
      border-top: 1px solid var(--header-border);
      justify-content: space-around;
      align-items: center;
    }

    @supports not ((backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px))) {
      .mobile-tab-bar {
        background-color: var(--card-bg);
      }
    }

    .tab-link {
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
      gap: 0.15rem;
      min-height: 2.75rem;
      padding: 0.25rem 0.2rem;
      border: 0;
      background: transparent;
      color: var(--nav-inactive);
      font-size: var(--text-2xs);
      font-weight: 500;
      text-decoration: none;
      cursor: pointer;
      -webkit-tap-highlight-color: transparent;
      transition: color 0.15s ease;
    }

    .tab-link:hover,
    .tab-link.active {
      color: var(--primary);
    }

    .tab-action .tab-action-icon {
      display: grid;
      place-items: center;
      width: 2rem;
      height: 2rem;
      border-radius: var(--radius-md);
      background-color: var(--primary-soft);
      color: var(--primary);
      transition: transform 0.15s ease, background-color 0.15s ease;
    }

    .tab-action:active .tab-action-icon {
      transform: scale(0.92);
      background-color: var(--primary);
      color: var(--text-inverse);
    }

    .tab-label {
      line-height: 1;
    }

    .bar {
      padding-inline: var(--space-5);
    }

    .omni-trigger {
      display: none;
    }

    .ask-btn-text {
      display: none;
    }

    .meta {
      gap: 0.15rem;
    }

    .meta .btn {
      min-width: 2.75rem;
      min-height: 2.75rem;
      padding: 0.5rem;
    }

    nav.mobile {
      width: min(19rem, 100%);
    }

    main {
      padding: var(--space-5);
    }

    footer {
      padding-inline: 1rem;
    }
  }
</style>
