<script lang="ts">
  import { onMount } from "svelte";
  import FluidSymphony from "$lib/symphony/FluidSymphony.svelte";
  import SymphonyBar from "$lib/symphony/SymphonyBar.svelte";
  import SoundscapePeek from "$lib/symphony/SoundscapePeek.svelte";
  import { symphonyStore } from "$lib/symphony/symphonyStore.svelte";

  type Chrome = "bar" | "peek" | "full";

  /**
   * Decided before the first render, not in onMount: this route is `ssr = false`,
   * so the query is available immediately — and a mode picked one render late
   * means the full surface mounts, runs, and is torn down again for nothing.
   */
  const query = new URLSearchParams(location.search);

  /** The embedding origin, when there is one. Also the only origin we talk to. */
  const parentOrigin = readParentOrigin();

  function readParentOrigin(): string | null {
    const parent = query.get("parent");
    // Only ever an origin, and only when we really are framed: a page that trusts
    // a parent it does not have is a page that trusts anyone who frames it.
    if (!parent || window.self === window.top) return null;
    try {
      return new URL(parent).origin;
    } catch {
      return null;
    }
  }

  const requestedChrome = query.get("chrome");
  let chrome = $state<Chrome>(
    requestedChrome === "bar" || requestedChrome === "peek" ? requestedChrome : "full",
  );

  const embedded = parentOrigin !== null;

  /**
   * Ask the embedder to resize the frame. The mode itself is never changed by
   * navigating — reloading this document would take the AudioContext with it, and
   * the whole point of the dock is that the sound survives.
   */
  function requestChrome(mode: Chrome) {
    if (!parentOrigin) return;
    window.parent.postMessage({ type: "soundscape:chrome", mode }, parentOrigin);
  }

  onMount(() => {
    // The label is what other surfaces show as "playing where".
    symphonyStore.connect(parentOrigin ? "dashboard" : "panel");

    const onMessage = (event: MessageEvent) => {
      if (!parentOrigin || event.origin !== parentOrigin) return;
      const data = event.data;
      if (data?.type !== "soundscape:chrome") return;
      if (data.mode === "bar" || data.mode === "peek" || data.mode === "full") chrome = data.mode;
    };
    window.addEventListener("message", onMessage);

    return () => {
      window.removeEventListener("message", onMessage);
      symphonyStore.disconnect();
    };
  });

  function onKeyDown(event: KeyboardEvent) {
    if (event.key === "Escape" && embedded && chrome !== "bar") requestChrome("bar");
  }
</script>

<svelte:window onkeydown={onKeyDown} />

<svelte:head>
  <title>Soundscape — Sjel</title>
  <meta name="description" content="Generative audio, composed in the browser, conducted by the capability" />
</svelte:head>

{#if chrome === "bar"}
  <div class="dock">
    <SymphonyBar
      onpeek={embedded ? () => requestChrome("peek") : undefined}
      onexpand={embedded ? () => requestChrome("full") : undefined}
    />
  </div>
{:else if chrome === "peek"}
  <div class="peek">
    <SoundscapePeek
      oncollapse={() => requestChrome("bar")}
      onexpand={() => requestChrome("full")}
    />
  </div>
{:else}
  <div class="page">
    <FluidSymphony
      showOverlay={!embedded}
      oncollapse={embedded ? () => requestChrome("bar") : undefined}
    />
  </div>
{/if}

<style>
  .page {
    position: fixed;
    inset: 0;
    width: 100vw;
    height: 100vh;
    background: #05030a;
  }

  .dock {
    position: fixed;
    inset: 0;
    height: 100vh;
  }

  .peek {
    position: fixed;
    inset: 0;
    height: 100vh;
    background: #0a0a0c;
  }
</style>
