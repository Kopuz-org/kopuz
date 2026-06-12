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

## Engine TDD loop (2026-06-13) — new methodology
All 0.2.4-era workarounds REVERTED (recoverable: checkpoint commit 9bb9b3e,
reverts c7e9086 + 7069e28..ce3bdb1). Engine bugs now get fixed in the local
blitz checkout (~/projects/blitz, path dep) with red→green tests, verified
live in kopuz. No kopuz-side workarounds for engine bugs anymore.

14. **Comment nodes force inline layout → whole pages invisible** (the
    "nothing on favorites/playlists" bug; Home escaped only by being the
    initial route... actually by markup: its root is normal-flow). A Dioxus
    conditional renders a COMMENT placeholder; in blitz main's
    collect_layout_children a comment has no styles → defaults to
    display:inline → container with [comment, abspos div] classified
    all_inline → becomes inline root → element children embedded in parley
    inline layout as out-of-flow boxes sized 0x0. Diagnosed via Alt+T taffy
    dump (wrapper 0x0, "BOX OutOfFlow") + Alt+Y node dump (temp keybind in
    blitz-shell, uncommitted). Fixed in blitz branch
    fix/comment-inline-classification (65ed76a): skip comments in the
    classification scan + regression tests
    (packages/blitz-html/tests/comment_layout.rs). Latent secondary bug for
    upstream: out-of-flow boxes inside GENUINE inline contexts still zero-
    size.
    Debug tooling learned: blitz windows respond to Alt+D (layout overlay),
    Alt+H (hover highlight), Alt+T (taffy tree dump to stdout).
    Also: vello-hybrid (main's default backend) panics on 0-alpha paints
    ("Color fields with 0 alpha are reserved for clipping") — kopuz uses
    the "vello" feature instead; ICU4X lacks the ja segmentation model
    (cjdict) → stderr noise per Japanese title, non-fatal.

15. **display:contents fixed in the engine** (blitz a8d66e6) — three bugs:
    classification didn't recurse into contents children (all_inline
    misroute → inline root), the Contents hoisting arm pushed grandchildren's
    layout children (skipping a generation; leaves vanished), and the
    whitespace-anonymous-block cleanup pop()'d the wrong layout_children
    entry leaving a stale slab id (panic). With these + the comment fix the
    ORIGINAL kopuz markup runs under blitz: route wrapper + showcase rows
    reverted to plain class="contents" — the last structural kopuz
    workarounds are deleted.
16. **Radio cards 1px tall → taffy bug** (the real F10): taffy's block
    layout passes available_space.height = MinContent ("min-content
    constraint") for in-flow children when it means "indefinite" — every
    grid nested under an auto-height block then zeroes scroll-container
    items (auto min = 0) and the maximize step has zero free space.
    One-line fix (MinContent→MaxContent) in a local taffy checkout
    (~/projects/taffy-fix, [patch.crates-io] in BOTH blitz and kopuz
    workspaces). WPT css-grid+flexbox: +8 new passes / -2 regressions
    (fr-row tests — taffy's AvailableSpace can't distinguish "indefinite"
    from "max-content constraint"; needs an upstream design decision).
    Diagnosed with taffy's "debug" feature (algorithm trace) + pure-taffy
    repro at /tmp/taffy-repro.

17. **Favorites wouldn't scroll → abspos hoisted through contents was
    content-sized** (blitz a63f0a1): the all_out_of_flow early-return in
    collect_layout_children wasn't contents-transparent (pushed the
    contents node as a layout box; the abspos page laid out against it at
    0x50517 — no scroll capacity anywhere), and the classification cleared
    all_out_of_flow for contents children (transparency = no vote; the
    recursion already visits their children). Extracted
    push_hoisted_children_and_pseudos, reused in the Contents arm (which
    gained ::before/::after support). WPT grid+flexbox: 1154 passes before
    and after. Favorites/Library windowed lists scroll under blitz with
    the ORIGINAL markup.

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

18. **Button text left-aligned → UA stylesheet gap** (blitz 925926d): every
    browser's UA sheet has `button { text-align: center }`; blitz's
    default.css didn't. One line + a centering test (probe = inline-block
    embedded box position; plain inline spans get no box assigned).

## Upstreamed (2026-06-13)
PRs: blitz#452 (comment classification), blitz#453 (display:contents,
draft/stacked), blitz#454 (UA button centering), taffy#964 (block available
space — taffy's full suite incl. 4409 fixtures green; carries the
AvailableSpace "indefinite vs max-content constraint" design question).
Issues: blitz#455 (group-hover invalidation), #456 (range input), #457
(::before re-resolution), #458 (paint order), #459 (ICU ja/cjdict),
vello#1707 (vello_hybrid 0-alpha panic); impact comment on blitz#252
(hover media). Forks: UMCEKO/blitz, UMCEKO/taffy.
