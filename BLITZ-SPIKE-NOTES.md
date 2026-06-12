# blitz-spike findings (2026-06-12)

`KOPUZ_BLITZ=1 cargo run` from the repo root (assets resolve cwd-relative)
launches the dioxus-native (Blitz: Stylo + Taffy + Vello/wgpu) renderer;
plain `cargo run` keeps the webview. dioxus 0.7.9 → dioxus-native 0.7.9 →
blitz-dom 0.2.4. `image` workspace-pinned to =0.25.6 for blitz-dom.

## Works (more than expected)
- App shell: dark theme, sidebar + icons (FontAwesome via CDN — blitz `net`
  fetches remote CSS/fonts), nav highlight, player bar, LOCAL/SERVER toggle.
- Cover images over https (YT googleusercontent) — only `artwork://` (local
  files, wry custom protocol) is dead by design.
- Japanese text, flex/grid layout broadly, click events (sidebar nav works;
  blitz logs "Clicked link without href" for <a>-as-button, harmless).
- Whole backend: DB, scan, sync, Discord, radio — renderer-agnostic.

## Solved during the spike (each bisected with /tmp/blitz-scroll-repro)
1. **Scrolling** — `display: contents` contributes ~zero scrollable size in
   blitz-dom 0.2.4 (repro: scroller wrapping a contents-div clamps to ~1px).
   Kopuz's route shell used class="contents" → no scroll app-wide. Bypassed
   under blitz (3124430).
2. **Pages invisible** (everything with an absolute-inset root) — neither
   flex-1 (block parent) nor min-h-full (percentage min-height) gave the
   route wrapper a height in Taffy; pages anchored to a collapsed box.
   Wrapper is now absolute inset-0 against the relative main scroll area and
   carries its own overflow (402cad0). App functional after this.
3. **Hover styles** — Tailwind v4 wraps hover: variants in
   @media (hover: hover); blitz's Stylo device doesn't claim hover
   capability so all 60 rules dropped. `@custom-variant hover (&:hover)` in
   tailwind source emits plain pseudo-classes (c98c52a). Blitz's own :hover
   state machinery was fine.

## Solved, round two
4. **Source toggle** — three gaps in one widget: z-index on static flex items
   not honored (labels under the slider → make them `relative`),
   calc-percentage abspos widths mis-sized (→ inset pairs), abspos auto-top
   static position mis-resolved in a centered flex row (→ explicit `top-1`).
5. **Button text left-aligned** — blitz's UA sheet doesn't center button text
   like browsers; added to the base resets in main.css.
6. **Font** — nothing on the system is actually named "JetBrains Mono" (only
   Nerd Font variants); fontconfig substituted Noto Sans Mono in the webview,
   fontique (no substitution) fell to sans. Real family names in the stack
   fix BOTH renderers.
7. **SVG paints** — blitz hands SVG attrs to usvg, which can't evaluate
   var()/color-mix(): the settings EQ editor rendered black and warned per
   drag-frame. Resolved literals fix it.
8. **Segfault in NVIDIA vkCreateBuffer** (610.43.02, sane 438KB request,
   garbage top stack frames) fired on the Settings page and STOPPED
   REPRODUCING once the SVG paints were fixed — suggests the usvg/vello
   image path corrupts state upstream of the driver. Core dumps preserved
   via coredumpctl if a report needs them.
9. **Home hero overlay invisible** — paint-order gap: a positioned (relative,
   z-auto) sibling does not paint above an earlier abspos sibling; per spec
   both sit in the same paint level and the later one wins. Explicit
   `z-index` on the overlay fixes it (repro cases A/B blank, C visible);
   `z-10` added unconditionally (no-op in the webview). object-fit: cover +
   overflow clipping themselves verified fine in isolation (repro D).
   Same fix applied to the local home hero (local/home.rs).
10. **Listen Now rows collapse to nothing** — an `overflow: hidden` flex
    container as a grid-track child contributes zero content size, so the
    row gets 0 height and the clip hides everything (repro: identical row
    renders the moment overflow-hidden is dropped; same row with simple
    children still blank). Same family as finding 1 (scroll-size). Fix:
    blitz-conditional class without overflow-hidden (cost: cover corners
    not clipped to the rounded-2xl container under blitz). Swept the whole
    UI for the pattern (auto-height overflow-hidden child of a grid):
    three sites — server home Listen Now, local home Listen Now, radio
    Normal-layout station cards — all fixed; every other overflow-hidden
    has explicit height/aspect or sits outside a grid.

## Solved, round three (player controls)
11. **group-hover never fires** — blitz applies :hover state to layout
    ancestors but does NOT restyle descendants whose rules depend on an
    ancestor's hover (plain `.grp:hover .tgt` fails the same way as
    Tailwind's nested `:is(:where(.group):hover *)` form; hover states can
    also visibly stick). Consequence: every hover-revealed control was
    unreachable, and `hidden group-hover:flex` play buttons (TrackRow rows
    with numbers) weren't even hit-testable — clicks fell through to the
    row. Fix at one altitude: a blitz-only stylesheet in main.rs pins the
    hovered state permanently (`group-hover:opacity-100`→1, `:flex`→flex,
    `:inline-block`→inline-block, `:hidden`→none, `:opacity-0`→0).
    Hit-testing itself verified CORRECT (invisible abspos button receives
    clicks, stop_propagation works).
12. **`input type=range` is inert** — blitz-dom 0.2.4 only implements click
    interaction for checkboxes; seek + volume sliders (invisible range
    overlays) were dead. No coordinate fallback possible either:
    element_coordinates/page_coordinates and onmounted/get_client_rect are
    all unimplemented!() in dioxus-native 0.7.9 (only client_coordinates
    works). Workaround: BlitzSliderOverlay (components/slider_overlay.rs) —
    N invisible flex segments over the track, each mapping to a fixed
    fraction via plain onclick. Wired into normal+modern bottombars and
    fullscreen (seek 40 segments, volume 20). Track onwheel volume still
    works natively.
13. **Dynamic class swap on `<i>` doesn't update the FontAwesome glyph**
    (::before content not re-resolved on class change; sibling text updates
    fine). Fix: swap the element, not the class — if/else rsx branches
    (distinct templates → node replacement). Applied to play/pause, mute,
    and heart icons in both bottombars + fullscreen.

## Still broken
1. **Some clickable elements** — details TBD (next session: enumerate which
   clicks dead vs working; blitz logs "Clicked link without href" for
   <a>-as-button patterns, dropdown/context-menu behavior unverified).
2. **No text truncation** (`truncate` → ellipsis/nowrap/hidden) — card
   titles spill.
3. Misc CSS polish (paddings, radii, native checkbox/range widgets — e.g.
   the EQ preamp range slider in settings is still inert under blitz).

## Gotchas learned
- Assets resolve via CARGO_MANIFEST_DIR at runtime in dev: `cargo run` from
  the repo root works; invoking target/debug/kopuz directly loses all CSS.
- Blitz windows: empty class, title "Dioxus App" (for hyprctl/grim hunting).
- Repro harness lives at /tmp/blitz-scroll-repro (dioxus-native 0.7.9,
  cached deps — rebuilds in ~1s; bisect any suspected CSS gap there first).

## Deliberately broken on this branch
YT playback (pot minter + decipher need a JS engine — host-runtime plan in
~/blog-the-engine-was-already-there.md), custom titlebar, close-time persist
flush (webview-only wry handler).

## Re-test against blitz git main (2bcde3a1, 2026-06-12)
Harness: /tmp/blitz-repro-08 — dioxus 0.7.9 + dioxus-native via git. Key
discovery: blitz main targets dioxus 0.7.3, NOT the 0.8 alpha — the latest
engine is adoptable as a git dep with no dioxus migration. (The published
dioxus-native 0.8.0-alpha.0 does NOT build: vello_hybrid semver drift.)
- F10 overflow-hidden grid collapse: FIXED → the three conditional-class
  sites (server/local Listen Now, radio cards) are deletable on upgrade.
- F9 overlay paint order: still broken (z-10 fix stays; it's spec-neutral).
- F11 group-hover: now stuck always-visible instead of never — functionally
  identical to our override stylesheet, which becomes redundant-but-harmless.
- F12 range input: still inert, and no longer even rendered → slider
  overlay workaround stays.
- F1 display:contents: different bug (children flow inline) — bypass stays.
- truncate: clips without spilling now, still no ellipsis.
All still-broken rows confirmed reproducible on main → ready to file
upstream with the harness code.

## Next experiments
- File the still-broken findings (F9, F11, F12, F1, truncate, glyph swap)
  at DioxusLabs/blitz with the /tmp/blitz-repro-08 repro cases.
- Consider git-main blitz in kopuz (no dioxus bump needed) to delete the
  F10 conditional classes.
- Consolidation pass for code quality: shared SeekSlider/VolumeSlider
  components (deletes pre-existing 3x duplication AND hides the blitz
  branch in one place), centralize remaining blitz conditionals.
- Local-file covers: serve over localhost HTTP instead of artwork:// (works
  for both renderers, deletes the wry protocol).
