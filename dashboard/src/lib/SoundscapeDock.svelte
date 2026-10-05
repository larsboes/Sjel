<script lang="ts">
  import { onMount } from "svelte";
  import { capabilities } from "./capabilities.svelte";
  import { hasPanel, panelUrl } from "./api";

  /**
   * The soundscape panel, embedded once and never remounted.
   *
   * Web Audio lives in a page: it cannot be handed from one document to another, and
   * two documents playing the same scape is the defect #113 fixed. So the spine does
   * not rebuild the player, it hosts the real one. SvelteKit swaps only the page
   * slot, so this frame — and the AudioContext inside it — survives navigation
   * between dashboard pages. A full reload still stops the sound; nothing can change
   * that, and the conductor restores the state a click away.
   */

  type Chrome = "bar" | "peek" | "full";

  let chrome = $state<Chrome>("bar");
  let frame = $state<HTMLIFrameElement | null>(null);
  /** Client-only: this file prerenders, and there is no location then. */
  let origin = $state<string | null>(null);
  let panelReachable = $state(false);

  const soundscape = $derived(capabilities.byName("soundscape"));
  const available = $derived(
    panelReachable && soundscape !== undefined && hasPanel(soundscape) && soundscape.up !== false,
  );

  /**
   * Set once and never again. It cannot be derived from the registry: that is a
   * poll, and a single sample reporting the capability as down would tear the
   * frame out and take a running AudioContext with it. Reloading the document is
   * the one thing this component exists to avoid.
   */
  let src = $state<string | null>(null);

  $effect(() => {
    if (src || !available || !origin) return;
    src = `${panelUrl(soundscape!)}?chrome=bar&parent=${encodeURIComponent(origin)}`;
  });

  const panelOrigin = $derived(src ? new URL(src).origin : null);

  /**
   * The shell owns the geometry, so it also owns the answer. The frame only asks;
   * without telling it what was decided, it would keep rendering the bar inside a
   * full-height box — which is exactly what a missing reply looks like.
   */
  function setChrome(mode: Chrome) {
    chrome = mode;
    if (panelOrigin) frame?.contentWindow?.postMessage({ type: "soundscape:chrome", mode }, panelOrigin);
  }

  onMount(() => {
    origin = location.origin;

    // Soundscape deliberately binds to loopback. A shell reached through Tailscale
    // cannot load the panel's separate port; mounting that failed iframe leaves a
    // blank dock in mobile Safari. Keep the embedded player local until the panel has
    // a real same-origin remote route.
    panelReachable =
      location.protocol === "http:" &&
      (location.hostname === "127.0.0.1" || location.hostname === "localhost");

    const onMessage = (event: MessageEvent) => {
      // Only the frame we embedded gets to resize itself.
      if (!panelOrigin || event.origin !== panelOrigin) return;
      const data = event.data;
      if (data?.type !== "soundscape:chrome") return;
      if (data.mode === "full" || data.mode === "peek" || data.mode === "bar") setChrome(data.mode);
    };

    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  });

  // The shell reads this to reserve room, so the bar covers nothing and no page
  // has to know whether the capability is running.
  $effect(() => {
    if (!src) return;
    const root = document.documentElement;
    root.style.setProperty("--soundscape-dock-height", "56px");
    return () => root.style.removeProperty("--soundscape-dock-height");
  });

  // Escape closes the overlay, the same as anywhere else in the shell. The frame
  // keeps its own keys; this only fires when the outer document has focus.
  function onKeyDown(event: KeyboardEvent) {
    if (event.key === "Escape" && chrome !== "bar") setChrome("bar");
  }
</script>

<svelte:window onkeydown={onKeyDown} />

{#if src}
  <div class="dock" class:peek={chrome === "peek"} class:full={chrome === "full"}>
    <iframe
      bind:this={frame}
      {src}
      title="Soundscape"
      allow="autoplay"
      loading="eager"
    ></iframe>
  </div>
{/if}

<style>
  .dock {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    height: var(--soundscape-dock-height, 56px);
    z-index: 40;
    transition: height var(--motion-slow) cubic-bezier(0.4, 0, 0.2, 1);
  }

  .dock.peek {
    height: min(330px, calc(100vh - 24px));
    z-index: 50;
  }

  .dock.full {
    top: 0;
    height: 100vh;
    z-index: 60;
  }

  iframe {
    display: block;
    width: 100%;
    height: 100%;
    border: 0;
  }

  @media (prefers-reduced-motion: reduce) {
    .dock {
      transition: none;
    }
  }
</style>
