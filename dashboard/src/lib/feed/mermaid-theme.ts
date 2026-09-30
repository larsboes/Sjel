/**
 * "Warm scientific teal" — the diagram palette, in one place.
 *
 * Mermaid's stock theme picks saturated yellow, pink and violet per branch and
 * a hard blue root. It reads as a debug view, not as a figure. This is the
 * palette the operator's written work already uses, so a diagram in the reader
 * and a diagram in a paper look like they came from the same hand.
 *
 * `roots` are the only literal colours in this file; every theme variable below
 * refers to one of them. Adding a colour means adding a root, which is what
 * stops the palette quietly growing a fourth teal.
 */

export const ROOTS = {
  ink: "#1F2727",
  paper: "#FBFAF7",
  white: "#FFFFFF",
  tealDark: "#4A6A69",
  tealMid: "#85A09F",
  aquaLight: "#A4C5C4",
  tealLight: "#E6EFED",
  warmSand: "#C7A183",
  warmMauve: "#9D8C8C",
  warmDark: "#8A6653",
  warmLight: "#F3E9E4",
  blueDark: "#527B8B",
  blueLight: "#E7EFF1",
  grid: "#DDD6CF",
  mist: "#F3F5F3",
} as const;

/**
 * The per-branch ramp. A mindmap and a pie do not read `primaryColor` at all —
 * they walk `cScale0..n`, and left unset Mermaid derives that ramp by rotating
 * hue off the primary, which is how a teal palette produced violet branches.
 * Naming the ramp is the only way the palette actually governs those diagram
 * types.
 *
 * Alternating warm/cool and mid/pale rather than running light-to-dark. A ramp
 * sorted by lightness put two near-white rungs next to each other, and on a
 * four-branch mindmap those two branches stopped being distinguishable at all.
 * Adjacent rungs differ in both hue family and weight, so any branch count from
 * two to eight stays readable.
 */
const SECTION_SCALE = [
  ROOTS.aquaLight,
  ROOTS.warmSand,
  ROOTS.blueLight,
  ROOTS.warmMauve,
  ROOTS.tealLight,
  ROOTS.warmLight,
  ROOTS.tealMid,
  ROOTS.mist,
] as const;

/** A border per rung, so the pale ones still have an edge to sit on. */
const SECTION_BORDER = [
  ROOTS.tealDark,
  ROOTS.warmDark,
  ROOTS.blueDark,
  ROOTS.warmDark,
  ROOTS.tealDark,
  ROOTS.warmSand,
  ROOTS.tealDark,
  ROOTS.grid,
] as const;

/** `cScale0..n`, its label and its border, all bound to the palette. */
function sectionVariables(): Record<string, string> {
  const scale: Record<string, string> = {};
  SECTION_SCALE.forEach((fill, index) => {
    scale[`cScale${index}`] = fill;
    // Ink on every rung: the ramp above is chosen so dark text always holds,
    // which is what lets the label colour be one decision instead of eight.
    scale[`cScaleLabel${index}`] = ROOTS.ink;
    scale[`cScalePeer${index}`] = SECTION_BORDER[index];
  });
  return scale;
}

/**
 * Node fills and text are identical in both modes, deliberately: they carry the
 * diagram's identity, and a reader who toggles the theme should see the same
 * figure rather than a different one. Only what sits *behind* the figure adapts,
 * plus the line colour — `tealDark` on a dark card is nearly invisible, so dark
 * mode steps one rung up the same teal.
 */
function themeVariables(dark: boolean) {
  return {
    ...sectionVariables(),
    primaryColor: ROOTS.aquaLight,
    primaryTextColor: ROOTS.ink,
    primaryBorderColor: ROOTS.tealDark,
    secondaryColor: ROOTS.blueLight,
    secondaryTextColor: ROOTS.ink,
    secondaryBorderColor: ROOTS.blueDark,
    tertiaryColor: ROOTS.paper,
    tertiaryTextColor: ROOTS.ink,
    tertiaryBorderColor: ROOTS.warmSand,
    lineColor: dark ? ROOTS.tealMid : ROOTS.tealDark,
    clusterBkg: ROOTS.paper,
    clusterBorder: ROOTS.warmSand,
    edgeLabelBackground: dark ? ROOTS.paper : ROOTS.white,
    background: dark ? ROOTS.ink : ROOTS.white,
    mainBkg: ROOTS.aquaLight,
    nodeBorder: ROOTS.tealDark,
    textColor: ROOTS.ink,
    fontSize: "16px",
    // The house stack rather than the paper's Arial: the colours are what make
    // this the same palette, and a foreign font inside the reader is just a
    // foreign font.
    fontFamily: "var(--font-sans)",
  };
}

/**
 * Thin strokes and a flat edge curve. Mermaid's defaults are heavier, which on
 * a fifteen-node mindmap turns the links into the loudest thing on screen.
 */
const THEME_CSS = [
  ".node rect, .node polygon, .node circle, .node path { stroke-width: 1.5px; }",
  ".flowchart-link, .edgePath .path { stroke-width: 1.5px; }",
  ".marker { stroke-width: 1.5px; }",
].join(" ");

/**
 * `layout` and `look` are pinned, and they are the whole visual surface of the mermaid 12
 * upgrade rather than caution for its own sake.
 *
 * 12.0.0 bundles ELK and makes it the default layout, and makes `neo` the default look:
 * "Existing flowcharts, state and class diagrams will re-lay out and recolour; add
 * `layout: dagre`, `theme: default` and `look: classic` to your config to keep the old look"
 * (mermaid@12.0.0 release notes, 2026-09-10). `theme` stays `base` here, because the palette
 * above is this repository's rather than mermaid's. The other two are exactly the settings
 * that would restyle every diagram in the reader with nobody looking at one.
 *
 * `layout` is the sharper edge of the two. The `flowchart.nodeSpacing` and `rankSpacing` below
 * are dagre's knobs and ELK does not read them -- it has its own `elk.*` spacing -- so moving
 * to ELK is not a layout swap that leaves this file alone. It is a re-tune plus a look at
 * every figure, which is a review, not a config default to drift into.
 *
 * Deleting these two lines is the first step of doing that deliberately.
 */
export function mermaidConfig(dark: boolean) {
  return {
    startOnLoad: false,
    securityLevel: "strict" as const,
    theme: "base" as const,
    layout: "dagre" as const,
    look: "classic" as const,
    themeVariables: themeVariables(dark),
    themeCSS: THEME_CSS,
    flowchart: {
      curve: "linear" as const,
      htmlLabels: true,
      nodeSpacing: 34,
      rankSpacing: 38,
    },
  };
}
