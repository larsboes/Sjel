<script lang="ts">
  /**
   * The icons this shell uses, inline.
   *
   * A dependency would buy a few thousand glyphs to ship a dozen, and every dependency
   * needs an upstreams.toml verdict before it may be consumed
   * (CONTRIBUTING.md#dependency-verdicts-and-provenance and CONTRIBUTING.md#patch-first). Paths are
   * from Lucide (ISC), which the React shell used;
   * inlining them keeps the attribution honest and the bundle self-contained.
   */
  type Name =
    | "home" | "feed" | "boxes" | "graduation" | "server" | "compass" | "train"
    | "map-pin" | "database" | "external" | "arrow-right" | "play" | "square"
    | "refresh" | "sun" | "moon" | "menu" | "close" | "clock" | "alert" | "wifi-off"
    | "check" | "loader" | "plus" | "search" | "swap" | "calendar" | "ticket"
    | "git-branch" | "thermometer" | "cpu" | "activity" | "chevron" | "mail" | "globe"
    | "layout" | "wallet" | "history" | "sparkles" | "message" | "send" | "users" | "pencil";

  let { name, size = 16 }: { name: Name; size?: number } = $props();

  const paths: Record<Name, string> = {
    "users": "M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2 M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8 M22 21v-2a4 4 0 0 0-3-3.87 M16 3.13a4 4 0 0 1 0 7.75",
    "home": "M3 9l9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z M9 22V12h6v10",
    // Vier Felder statt eines Hauses: die Seite zeigt einen Grundriss, und /"/" ist schon home.
    "layout": "M3 3h7v9H3z M14 3h7v5h-7z M14 12h7v9h-7z M3 16h7v5H3z",
    "feed": "M4 11a9 9 0 0 1 9 9 M4 4a16 16 0 0 1 16 16 M5 19a1 1 0 1 0 0-2 1 1 0 0 0 0 2",
    // Three isometric cubes, all three CLOSED and all three inside the 24x24 box. The
    // third subpath used to read `M12 8.5 8 6V2l4-2.5`, which walks to y = -0.5 and never
    // closes — so the More menu and the Capabilities nav item drew an open top box with a
    // stroke leaving the viewBox.
    "boxes": "M7 16.5 3 14v-4l4-2.5L11 10v4z M17 16.5 13 14v-4l4-2.5L21 10v4z M12 10.5 8 8V4l4-2.5L16 4v4z",
    "graduation": "M22 10 12 5 2 10l10 5z M6 12v5c3 3 9 3 12 0v-5",
    "server": "M4 4h16v6H4z M4 14h16v6H4z M8 7h.01 M8 17h.01",
    "compass": "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z M16.2 7.8l-2.9 6.9-6.9 2.9 2.9-6.9z",
    "train": "M5 9h14 M8 19l-2 3 M16 19l2 3 M7 4h10a3 3 0 0 1 3 3v9a3 3 0 0 1-3 3H7a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3z M9 15h.01 M15 15h.01",
    "map-pin": "M20 10c0 6-8 12-8 12s-8-6-8-12a8 8 0 0 1 16 0z M12 12a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z",
    "database": "M12 8c4.4 0 8-1.3 8-3s-3.6-3-8-3-8 1.3-8 3 3.6 3 8 3z M4 5v14c0 1.7 3.6 3 8 3s8-1.3 8-3V5 M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3",
    "external": "M15 3h6v6 M10 14 21 3 M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6",
    "arrow-right": "M5 12h14 M12 5l7 7-7 7",
    "play": "M6 3l14 9-14 9z",
    "square": "M5 5h14v14H5z",
    "refresh": "M3 12a9 9 0 0 1 15-6.7L21 8 M21 3v5h-5 M21 12a9 9 0 0 1-15 6.7L3 16 M3 21v-5h5",
    "sun": "M12 17a5 5 0 1 0 0-10 5 5 0 0 0 0 10z M12 1v2 M12 21v2 M4.2 4.2l1.4 1.4 M18.4 18.4l1.4 1.4 M1 12h2 M21 12h2 M4.2 19.8l1.4-1.4 M18.4 5.6l1.4-1.4",
    "moon": "M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z",
    "menu": "M3 6h18 M3 12h18 M3 18h18",
    "close": "M18 6 6 18 M6 6l12 12",
    "clock": "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z M12 6v6l4 2",
    "alert": "M12 9v4 M12 17h.01 M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z",
    "wifi-off": "M2 2l20 20 M8.5 16.5a5 5 0 0 1 7 0 M5 12.9a10 10 0 0 1 5.2-2.7 M1.4 9.1a15 15 0 0 1 4.2-2.8 M22.6 9.1a15 15 0 0 0-9.6-3 M19 12.9a10 10 0 0 0-2-1.4 M12 20h.01",
    "check": "M20 6 9 17l-5-5",
    "loader": "M12 2v4 M12 18v4 M4.9 4.9l2.9 2.9 M16.2 16.2l2.9 2.9 M2 12h4 M18 12h4 M4.9 19.1l2.9-2.9 M16.2 7.8l2.9-2.9",
    "plus": "M12 5v14 M5 12h14",
    "search": "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16z M21 21l-4.3-4.3",
    "swap": "M8 3 4 7l4 4 M4 7h16 M16 21l4-4-4-4 M20 17H4",
    "calendar": "M8 2v4 M16 2v4 M3 10h18 M5 4h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z",
    "mail": "M4 5h16a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z M3.5 6.5 12 13l8.5-6.5",
    "ticket": "M3 9a3 3 0 0 1 0 6v3a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-3a3 3 0 0 1 0-6V6a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2z M13 5v2 M13 11v2 M13 17v2",
    "git-branch": "M6 3v12 M18 9a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M6 21a3 3 0 1 0 0-6 3 3 0 0 0 0 6z M15 6a9 9 0 0 1-9 9",
    "thermometer": "M14 14.76V3.5a2.5 2.5 0 0 0-5 0v11.26a4.5 4.5 0 1 0 5 0z",
    "cpu": "M9 5v-2 M15 5v-2 M9 21v-2 M15 21v-2 M5 9h-2 M5 15h-2 M21 9h-2 M21 15h-2 M7 7h10v10H7z",
    "activity": "M22 12h-4l-3 9L9 3l-3 9H2",
    "chevron": "M9 18l6-6-6-6",
    "wallet": "M20 9V7a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h13a2 2 0 0 0 2-2v-2 M22 9h-5a3 3 0 0 0 0 6h5z",
    "history": "M3 12a9 9 0 1 0 3-6.7L3 8 M3 3v5h5 M12 7v5l4 2",
    "globe": "M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20z M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20 M2 12h20",
    "sparkles": "M9.937 15.5A2 2 0 0 0 8.5 14.063l-6.135-1.582a.5.5 0 0 1 0-.962L8.5 9.936A2 2 0 0 0 9.937 8.5l1.582-6.135a.5.5 0 0 1 .963 0L14.063 8.5A2 2 0 0 0 15.5 9.937l6.135 1.581a.5.5 0 0 1 0 .964L15.5 14.063a2 2 0 0 0-1.437 1.437l-1.582 6.135a.5.5 0 0 1-.963 0z M20 3v4 M22 5h-4 M4 17v2 M5 18H3",
    "message": "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z",
    "send": "M22 2 11 13 M22 2 15 22 11 13 2 9z",
    "pencil": "M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z",
  };
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  class:spin={name === "loader"}
>
  {#each paths[name].split(" M") as segment, i (i)}
    <path d={i === 0 ? segment : `M${segment}`} />
  {/each}
</svg>

<style>
  svg {
    flex-shrink: 0;
    display: block;
  }

  .spin {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
</style>
