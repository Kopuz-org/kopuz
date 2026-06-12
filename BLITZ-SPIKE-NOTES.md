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

## Broken
1. **Scrolling: nothing scrolls.** blitz-dom DOES implement nested
   overflow-auto scrolling with bubbling (document.rs `scroll_node_by`), so
   the suspect is content-size measurement through kopuz's structure:
   `absolute inset-0` page wrappers + `display: contents` route shell +
   flex-column scrollers. If Taffy computes scroll_height=0 there, scrolling
   clamps to nothing everywhere. Next step: minimal repro (plain
   overflow-y-auto div with tall content, then add the absolute/contents
   wrappers until it breaks) → either fix kopuz's structure or file upstream.
2. **Hero overlay gone + cover unclipped/zoomed** — absolutely-positioned
   children inside the relative hero container don't render/lay out;
   object-fit/overflow clipping not applied.
3. **No text truncation** (`truncate` → ellipsis/nowrap/hidden) — card
   titles spill.
4. JetBrains Mono not picked up (falls back to default sans).
5. Minor: pill toggle styling, paddings, scroll chevrons, card height
   uniformity (follows from #3).

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
