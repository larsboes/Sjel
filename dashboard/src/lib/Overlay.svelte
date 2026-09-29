<script lang="ts">
  import type { Snippet } from "svelte";
  import { modal } from "./modal";

  let {
    title,
    eyebrow,
    onClose,
    busy = false,
    width = "520px",
    children,
  }: {
    title: string;
    eyebrow?: string;
    onClose: () => void;
    /** While a save is in flight, Escape and the backdrop stop closing the sheet. */
    busy?: boolean;
    /** Sheet width before the viewport caps it. A form with two columns asks for more. */
    width?: string;
    children: Snippet;
  } = $props();

  let touchStartY = 0;
  let touchDeltaY = $state(0);

  function handleTouchStart(e: TouchEvent) {
    if (e.touches.length === 1) {
      touchStartY = e.touches[0].clientY;
    }
  }

  function handleTouchMove(e: TouchEvent) {
    if (e.touches.length === 1) {
      const delta = e.touches[0].clientY - touchStartY;
      if (delta > 0) {
        touchDeltaY = delta;
      }
    }
  }

  function handleTouchEnd() {
    if (touchDeltaY > 90 && !busy) {
      onClose();
    }
    touchDeltaY = 0;
  }

  const titleId = `overlay-title-${Math.random().toString(36).slice(2, 9)}`;
</script>

<div class="overlay">
  <button class="backdrop" aria-label="Close dialog" onclick={() => !busy && onClose()}></button>
  <!-- Mount focus, the Tab trap and the focus restore all live in `$lib/modal.ts`. They
       were written here first and three other dialogs went without; the trap in
       particular is the same code, moved rather than rewritten. -->
  <div
    class="sheet"
    style={`--overlay-width: ${width}; ${touchDeltaY > 0 ? `transform: translateY(${touchDeltaY}px); transition: none;` : ''}`}
    use:modal={{ onClose, canClose: () => !busy }}
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
  >
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div
      class="mobile-drag-pill-wrap"
      ontouchstart={handleTouchStart}
      ontouchmove={handleTouchMove}
      ontouchend={handleTouchEnd}
      aria-hidden="true"
    >
      <div class="mobile-drag-pill"></div>
    </div>

    <div class="heading">
      <div>
        {#if eyebrow}<p class="eyebrow">{eyebrow}</p>{/if}
        <h2 id={titleId}>{title}</h2>
      </div>
      <button class="close" aria-label="Close dialog" onclick={onClose}>×</button>
    </div>

    {@render children()}
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 20px;
    overscroll-behavior: contain;
  }

  .backdrop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    border: 0;
    background: rgba(0, 0, 0, 0.48);
    cursor: default;
    animation: fade-in 0.15s ease-out;
  }

  .sheet {
    position: relative;
    width: min(var(--overlay-width), 100%);
    max-height: 90vh;
    max-height: 90dvh;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
    overscroll-behavior: contain;
    padding: 24px;
    border: 1px solid var(--card-border);
    border-radius: 14px;
    background: var(--card-bg);
    box-shadow: 0 16px 48px rgba(0, 0, 0, 0.35);
    transition: transform 0.2s cubic-bezier(0.16, 1, 0.3, 1);
  }

  .mobile-drag-pill-wrap {
    display: none;
    width: 100%;
    padding: 0 0 12px;
    cursor: grab;
    touch-action: pan-y;
    justify-content: center;
    align-items: center;
  }

  .mobile-drag-pill {
    width: 36px;
    height: 4px;
    border-radius: 9999px;
    background: var(--card-border);
  }

  .sheet:focus {
    outline: none;
  }

  .heading {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 18px;
  }

  /* Sentence case: the eyebrow says which section this sheet belongs to, and a
     tracked-out all-caps line above every heading is template chrome. */
  .eyebrow {
    margin: 0 0 3px;
    color: var(--text-secondary);
    font-size: var(--text-2xs);
    font-weight: 600;
  }

  h2 {
    margin: 0;
    font-size: 1.125rem;
  }

  .close {
    width: 30px;
    height: 30px;
    border: 0;
    border-radius: 50%;
    background: var(--surface);
    color: var(--text-primary);
    font-size: 1.25rem;
    line-height: 1;
    cursor: pointer;
  }

  @keyframes fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
  }

  @media (max-width: 560px) {
    .overlay {
      padding: 0;
      align-items: flex-end;
    }

    .mobile-drag-pill-wrap {
      display: flex;
    }

    .sheet {
      padding: 14px 20px 20px;
      padding-bottom: max(20px, env(safe-area-inset-bottom, 20px));
      border-radius: 18px 18px 0 0;
      max-height: 85dvh;
      border-bottom: none;
    }
  }
</style>
