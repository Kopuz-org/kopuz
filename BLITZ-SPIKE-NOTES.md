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

## Still broken (cosmetic tier)
1. **Hero overlay gone + cover unclipped/zoomed** — absolutely-positioned
   children inside the relative hero container don't render/lay out;
   object-fit/overflow clipping not applied.
2. **No text truncation** (`truncate` → ellipsis/nowrap/hidden) — card
   titles spill.
3. JetBrains Mono not picked up (falls back to default sans).
4. Minor: pill toggle styling, paddings, scroll chevrons, card height
   uniformity (follows from #2).

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

## Next experiments
- dioxus-native 0.8.0-alpha.0 exists (newer Blitz) — scroll/CSS gaps may
  already be fixed; needs dioxus 0.8 alpha workspace bump, try in isolation
  first.
- Minimal scroll repro against blitz-dom 0.2.4 → upstream issue with repro.
- Local-file covers: serve over localhost HTTP instead of artwork:// (works
  for both renderers, deletes the wry protocol).
