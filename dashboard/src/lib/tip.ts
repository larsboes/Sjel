// `use:tip={"Dismiss"}` — the tooltip, replacing the `title` attribute.
//
// `title` is the browser's tooltip, and it fails three ways a reader notices: it waits
// about a second before showing, it never shows on keyboard focus, and it never shows on
// touch at all. The dashboard carried 138 of them on 2026-10-05 and no other tooltip.
//
// One element serves every tip. It is a `popover="manual"`, so it sits in the top layer
// and no `overflow: hidden` card can clip it, and it is created on first use. The
// placement is script rather than CSS anchor positioning because one element follows many
// anchors, and the arithmetic is `placeTip` below, kept DOM-free so `bun test` can drive
// it (tools/dashboard-disclosure.test.ts).
//
// Accessibility: an element with no text of its own (an icon button) takes the tip as its
// `aria-label`; an element that has text takes it as `aria-description`. Either way a
// screen reader gets the words without having to hover.
//
// ponytail: no long-press on touch. A touch reader gets the label through the
// accessibility tree only; add a long-press when a tip carries something a touch reader
// needs and the row does not already show.

const SHOW_DELAY = 400;
/** After one tip closes, the next shows at once — the reader is already scanning. */
const WARM_WINDOW = 600;
const GAP = 6;
const EDGE = 8;

export type Rect = { top: number; left: number; width: number; height: number };

/** Where the tip goes: above the anchor, centred, flipped below when there is no room,
 *  and clamped so it never leaves the viewport sideways. */
export function placeTip(
  anchor: Rect,
  tip: { width: number; height: number },
  viewport: { width: number; height: number },
): { top: number; left: number } {
  const above = anchor.top - GAP - tip.height;
  const top = above >= EDGE ? above : anchor.top + anchor.height + GAP;
  const centred = anchor.left + anchor.width / 2 - tip.width / 2;
  const left = Math.min(Math.max(centred, EDGE), viewport.width - EDGE - tip.width);
  return { top, left: Math.max(left, EDGE) };
}

let bubble: HTMLElement | null = null;
let owner: HTMLElement | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
let warmUntil = 0;

function element(): HTMLElement {
  if (bubble) return bubble;
  bubble = document.createElement("div");
  bubble.id = "sjel-tip";
  bubble.setAttribute("popover", "manual");
  bubble.setAttribute("aria-hidden", "true");
  document.body.append(bubble);
  // The tip is fixed to the viewport, so a scroll would leave it pointing at nothing.
  window.addEventListener("scroll", () => hide(), { capture: true, passive: true });
  return bubble;
}

function show(target: HTMLElement, text: string): void {
  const tip = element();
  tip.textContent = text;
  if (!tip.matches(":popover-open")) tip.showPopover();
  const { top, left } = placeTip(
    target.getBoundingClientRect(),
    tip.getBoundingClientRect(),
    { width: window.innerWidth, height: window.innerHeight },
  );
  tip.style.top = `${top}px`;
  tip.style.left = `${left}px`;
  owner = target;
}

function hide(target?: HTMLElement): void {
  clearTimeout(timer);
  if (target && owner !== target) return;
  if (owner) warmUntil = Date.now() + WARM_WINDOW;
  owner = null;
  if (bubble?.matches(":popover-open")) {
    bubble.hidePopover();
  }
}

/** An empty value means no tip: `use:tip={row.reason}` on a row that has none is silent. */
export function tip(node: HTMLElement, text: string | null | undefined) {
  let current = text ?? "";
  // An aria-label the markup wrote is the author's; only one this action wrote may change.
  const ownLabel = !node.hasAttribute("aria-label");

  const label = () => {
    node.removeAttribute("title");
    if (!current) {
      node.removeAttribute("aria-description");
      if (ownLabel) node.removeAttribute("aria-label");
      return;
    }
    const hasText = (node.textContent ?? "").trim().length > 0;
    if (hasText) node.setAttribute("aria-description", current);
    else if (ownLabel) node.setAttribute("aria-label", current);
  };

  const enter = (event: PointerEvent) => {
    if (event.pointerType === "touch" || !current) return;
    clearTimeout(timer);
    const delay = Date.now() < warmUntil ? 0 : SHOW_DELAY;
    timer = setTimeout(() => show(node, current), delay);
  };
  const focus = () => {
    if (current && node.matches(":focus-visible")) show(node, current);
  };
  const leave = () => hide(node);
  const key = (event: KeyboardEvent) => {
    if (event.key === "Escape") hide(node);
  };

  label();
  node.addEventListener("pointerenter", enter);
  node.addEventListener("pointerleave", leave);
  node.addEventListener("pointerdown", leave);
  node.addEventListener("focus", focus);
  node.addEventListener("blur", leave);
  node.addEventListener("keydown", key);

  return {
    update(next: string | null | undefined) {
      current = next ?? "";
      label();
      if (owner !== node) return;
      if (current) show(node, current);
      else hide(node);
    },
    destroy() {
      hide(node);
      node.removeEventListener("pointerenter", enter);
      node.removeEventListener("pointerleave", leave);
      node.removeEventListener("pointerdown", leave);
      node.removeEventListener("focus", focus);
      node.removeEventListener("blur", leave);
      node.removeEventListener("keydown", key);
    },
  };
}
