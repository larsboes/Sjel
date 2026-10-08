# The 12 AI Vibe-Coded Design Tells (Detection & Fixes)

Derived from empirical audits of AI-generated web interfaces (`JCarterJohnson/vibecoded-design-tells`):

## 0. The "Tasteful AI Default" (Cream + Serif + Sage)
* **Code Signature**: Page backgrounds in `#faf8f5`, `#f5f1e8`, `bg-stone-50/100`, paired with display serifs (`Instrument Serif`, `Fraunces`, `Playfair Display`) and sage/forest green accents (`#15573a`, `emerald-800`).
* **Why it fails**: This is the single most recognized 2026 AI tell. It reads as algorithmic conformity masquerading as bespoke taste.
* **Fix**: Use Sjel's adopted IBM Plex typography and deliberate dark/light neutral scales without the faux-editorial cream tint.

## 1. Default shadcn / Unmodified Tailwind
* **Code Signature**: Standard zinc/slate color scales, default 0.5rem radius, generic card borders (`border border-zinc-200 dark:border-zinc-800`).
* **Fix**: Apply intentional density, custom border contrast, and distinct architectural accents tailored to Sjel's operational identity.

## 2. The AI Purple / Indigo Gradient
* **Code Signature**: Primary buttons and headers bathed in violet/indigo (`#6366f1`, `#8b5cf6`, `from-purple-600 to-indigo-600`).
* **Fix**: Use the tone tokens, each for its meaning (warning, success, danger, accent). No decorative gradients.

## 3. Gradient Text & Glowing Borders
* **Code Signature**: `bg-clip-text text-transparent bg-gradient-to-r`, pseudo-elements with `blur-xl bg-gradient-to-r`.
* **Fix**: Solid text with strong contrast. If emphasis is needed, use font weight, letter spacing, or subtle border treatment.

## 4. Too Many Animations / Bounce Overload
* **Code Signature**: Staggered `framer-motion` spring entrances on every table row, bouncing buttons, pulsing badges.
* **Fix**: Instant visual feedback for data changes. Micro-transitions must be fast (100–150ms ease-out) and purely functional.

## 5. Over-Rounded Corners
* **Code Signature**: `rounded-3xl` or `rounded-full` applied to data cards, modals, or rectangular content containers.
* **Fix**: Subtle, disciplined radius from the tokens (`--radius-sm`, `--radius-md`). Reserve `--radius-full` for compact chips and pills.

## 6. Neon Glow in Dark Mode
* **Code Signature**: Dark background `#09090b` with neon colored drop shadows (`shadow-[0_0_20px_rgba(59,130,246,0.5)]`).
* **Fix**: Elevation in dark mode should be represented through surface tinting (lighter shades of grey/neutral) and subtle 1px border lines, not colored neon halos.

## 7. Emojis as UI Icons
* **Code Signature**: Using 🚀, 💡, ⚡️, 📊 directly inside buttons, cards, or navigation items.
* **Fix**: Clean, monochrome SVG vector icons with consistent stroke weight (`Icon.svelte`).

## 8. Generic Sans-Serif Defaults
* **Code Signature**: Defaulting to `Inter`, `Geist`, or `Roboto` without font-feature-settings.
* **Fix**: Sjel adopts `IBM Plex Sans` for UI text and `IBM Plex Mono` with `tabular-nums` for data.

## 9. The "Hero + 3 Feature Cards" Skeletal Template
* **Code Signature**: Centered title, subtitle, two CTA buttons, followed by a 3-column grid of rounded cards with icons.
* **Fix**: Sjel interfaces are operational workspaces: organize by function, activity stream, horizon timelines, or inspectable tables.

## 10. Layout Quality Tells (Overflow & Padding Drift)
* **Code Signature**: Accidental horizontal scrollbars on mobile, padding that collapses awkwardly on resize, misaligned table numbers.
* **Fix**: Test responsive breakpoints, use `min-w-0` on flex children, and ensure numeric columns are right-aligned.

## 11. Low-Information Density
* **Code Signature**: Enormous whitespace where 4 lines of data consume an entire 1080p screen.
* **Fix**: Tighten padding, use compact typography, and present actionable information with high glanceability.
