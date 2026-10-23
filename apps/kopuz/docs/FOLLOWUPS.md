# Shared-layer follow-ups (apply AFTER all agents finish)

From page_library (a519fcea):
- [ ] `TrackList`: windowed mode — `set_window(rows, offset, total)` + emit
      `TrackListEvent::RangeChanged { offset, limit }` from the uniform_list range
      processor. Library and Activity both need it (§5.8). Note: with windowing,
      `TrackListEvent::Play(index)` must stay a *listing* index, not a row index.
- [ ] `TrackList`: a way to ENTER selection mode from a row (long-press or hover
      checkbox) — today only a page-level select-all can start it, so §3.9's
      long-press is unreachable.
- [ ] `TrackList`: `selected_count()` so pages don't build a Vec just to count.
- [ ] `TrackListOptions`: `now_playing` flag — §2.6 wants the highlight only when
      the queue IS this listing; today it always highlights by key.
- [ ] `store::actions`: `delete_tracks(keys, from_disk, cx)` helper; albums,
      playlists and favorites all want it.
- [ ] `components`: shared `SelectionBar` (§5.6 — every list page has the same one).
      page_library has a local one to lift.
- [ ] `en.ftl`: no key for "{n} selected" (hardcoded English in Dioxus too).

From page_downloader (af13edfe) — first SchemaForm consumer:
- [ ] `SchemaForm`: specs arrive async, so every consumer does `new(vec![], ..)`
      then `set_specs`, and `set_specs` needs `&mut Window` → forces `spawn_in` /
      `update_in` plumbing everywhere. Add `SchemaForm::empty(window, cx)` +
      `set_specs(specs, cx)` that builds InputStates lazily at render.
- [ ] `SchemaForm::set_specs` is destructive: rebuilds every InputState, dropping
      focus and uncommitted text. Diff on (key, kind, placeholder) and keep the
      existing input. Latent focus bug once a form has several text fields.
- [ ] `FieldChanged(pub FieldValue)` has a pub field, against our own rule; make it
      `value()`. Consider carrying the full `values()` snapshot too — some
      `set_*_settings` endpoints want the whole set.
- [ ] `FieldKind::Choice { custom }` is ignored, so §4.9's `custom_manual`
      free-text is missing. Harmless for the downloader (all `custom: false`) but
      sources/integrations use `custom: true` — settings_sources will hit this.
- [ ] Document that SchemaForm text fields answer on Blur/PressEnter only: a form
      inside a collapsible panel loses an edit if hidden while focused.
- [ ] `EmptyState` is `size_full`, so it overflows siblings inside a v_flex page
      unless wrapped in `flex_1().min_h_0()`. Add a flex-friendly variant.
- [ ] `components::icons`: no "sliders"/options or "gears"/processing glyph.

From page_activity + page_radio (ac29020c):
- [ ] `TrackList`: extra trailing columns (`Vec<(header, per-row element)>`) so a
      page like Activity (genre, plays) can use it instead of hand-rolling a row
      and losing the menu, selection, drag, highlight and cfg.show_row_images.
- [ ] `store`: a windowed-list helper (`Window<T>`), or make `Query::set_fetch`
      KEEP the stale value instead of dropping it — that drop is what forces every
      windowed page to keep its own rows/offset/total mirror. Activity hand-rolled
      it; Library skipped it. Unify before more pages copy either.
- [ ] `store`: a debounce helper — radio, search, the library filter and
      ConfigModel each hand-roll task-replacement debouncing.
- [ ] `api::Table` has no Radio entry, so radio queries only refresh on resync or
      source switch; pinning refreshes explicitly. Upstream daemon change if we
      want it.
- NOTE: toasts ARE wired — shell.rs subscribes to `Notices` and pushes
  notifications; the agent just could not verify it from inside its crate.
- ACCEPTED divergence: Activity plays via `QueueContext::Filter` rather than
  fetching every key first. Keep it.

SPACE KEY (fixed in app/src/main.rs, verify at integration):
- Bindings are matched before key listeners and `bindings_for_input` returns ALL
  matching bindings deepest-first, so a custom unhandled action would fall
  through to the Shell's space binding and still toggle playback. gpui's
  `NoAction` is the real escape hatch: it suppresses weaker bindings and
  dispatches nothing, so the character reaches the input.
  Bound `space -> NoAction` in the "Input" context.
- [ ] Remove page_search's local duplicate of this (it registers a
      process-wide binding from inside a page — works, but wrong home).

From page_search (ad20cd74):
- [ ] `TrackList`: no column sort/header at all — §2.2 and §2.6 both want it.
- [ ] `PageHeader`: no cover slot (genre detail wants the hero image).
- [ ] No scroll persistence (§5.7): TrackList owns its scroll handle privately
      and grids use plain overflow scroll, so positions reset on navigation.
- [ ] `Query`: no "don't run yet" mode; pages short-circuit inside the fetch.
- [ ] No "focus on route enter" hook, so search can't autofocus its input safely.

From page_discover (a1c78fcb):
- [ ] `Route::DiscoverPlaylist` should carry a `CatalogItemKind` (removes the
      album-retry, replaces the old "MPRE" id sniff properly).
- [ ] `PageHeader::back` takes a fixed Route; detail pages want
      `Navigator::go_back`. Add a back_button() variant.
- [ ] Lift the horizontal carousel (scroll handle + arrows + clamping) into
      `components::cards`; Home wants it too.
- [ ] `Tile`: add `on_click` — song cards need a command, not a route.
- [ ] `TrackList` is size_full+uniform_list, so embedding in a scrolling page
      needs a reserved height; expose ROW_HEIGHT or add an intrinsic-height mode.
- [ ] `DiscoverNowPlaying` should live in `store` to survive navigation.
- [ ] `en.ftl`: no plain `pause` key.
- [ ] No scroll-position paging primitive (every paged page re-rolls it).

From overlays (a90d7d49):
- [ ] Banners float over the page (covering its top ~28-40px) instead of taking
      layout space. Needs shell + `store::Chrome` cooperation — PRIORITISE.
- [ ] `DownloadsModel` stores statuses in a HashMap, losing daemon row order, so
      overlays re-fetches `download_statuses()` just to keep order. Keep the Vec.
- [ ] `PlayerModel::dismiss_error()` so surfaces agree on dismissal.
- [ ] `JobsModel`: add a `JobProgressed(JobProgress)` event so observers can
      filter by kind instead of waking on every job.

From settings_appearance + palettes (a3e9425b):
- [ ] **theme::apply_named only knows BUILT_IN**, so a `custom-*` id silently
      falls back to "default": the theme editor is WRITE-ONLY today. Needs a path
      building a Palette from `cfg.custom_themes[id].vars` (same 13 keys, infer
      `dark` from bg lightness). Same gap blocks the `album-art` and `live`
      pseudo-themes. theme/src/lib.rs is mine — PRIORITISE.
- [ ] `components`: a settings-row vocabulary (row/toggle/choice/slider/path + the
      locked hint). Three settings crates each built their own.
- [ ] `Button::disabled` does NOT disable `dropdown_menu` (the popover ignores the
      trigger's disabled flag), so a locked choice row still opens. Needs a shared
      helper; upstream bug in gpui-component.
- [ ] `ConfigModel` silently drops edits before load; add a loaded gate for
      settings pages instead of every writer remembering.
- NOTE: my stub palettes were WRONG (they carried the §5.12 theme-editor
  defaults, not the theme classes). Corrected by the agent; default is
  bg #000000 / copper accent.

From page_favorites (a449e9a4) — mostly CONFIRMING earlier requests:
- SelectionBar (3rd independent request), sortable TrackList header (3rd),
  scroll persistence (3rd), chained-Query helper (2nd).
- [ ] `TrackList`: expose `options()`; make it read `cfg.show_row_images` itself
      (§3.9) instead of each page wiring it.
- [ ] `TrackList::scroll_to` only does Center/non-strict; add `scroll_to_top()`.
- [ ] `TrackList` never installs a drag source, so `TrackDrag` is a type nothing
      emits — queue drop (§3.7) cannot work until it does.
- [ ] `PageHeader`: no icon slot (favorites heart dropped).
- [ ] `components::icons`: no sign-in glyph; set is curated so pages use raw paths.

RECURRING (implement first, in this order):
 1. SelectionBar in components
 2. Sortable header + column geometry exposed by TrackList
 3. Scroll persistence store (per route)
 4. TrackList drag source + windowed mode
 5. Chained/dependent Query helper
 6. theme::apply_named for custom themes (editor is write-only without it)
 7. Banner layout space (overlays covers the page top)

From settings_playback (a82359b1):
- [ ] **MY BUG in page_settings**: the 8-entry rail maps 4 categories onto one
      entity with no way to say which, so Offline/Metadata/Player/Tools all show
      the same stacked page (same for General/Appearance and Library/Connectivity).
      Fix: add a `show(category, cx)` to each of the three settings crates (safe
      to edit once their agents are done) and have the shell call it.
- CONFIRMED FIXED: SchemaForm's `managed_by_settings_file` key did not exist;
  already repointed to `setting_managed_by_system`.
- [ ] `store`: a one-line "edit one config field" helper — every settings row
      repeats `ConfigModel::global(cx).update(cx, |m, cx| m.update(cx, |c| ...))`.
- [ ] `SchemaForm`: add `specs()` getter + windowless `set_specs` so a save's
      authoritative response can be adopted without a refetch round-trip.
- NOTE: equalizer is sliders, not the drag-able curve graph (§4.7). Accepted.

From page_artists (ae99b324) — 4th/5th independent request for the same items:
- ViewModeToggle + SortControl + SelectionBar in components (4 crates will dup).
  `page_artists::controls::SortTarget` trait generalizes cleanly — lift it.
- PageHeader back-in-history variant (3rd request).
- [ ] `TrackListEvent::Play(index)` should carry the keys (or key) — every page
      re-reads `keys()` to rebuild the queue.
- [ ] `Cover`: no click/hover-overlay, so artwork upload is hand-assembled;
      albums needs the identical thing + trash. Add `EditableCover`.
- [ ] `Tile::route()` covers the whole tile, so a per-card dots menu double-fires
      navigation; add a `Tile::menu(..)` slot with its own click region. (This is
      why artists wrote its own card instead of using Tile.)
- [ ] `store::actions`: no library mutations — delete_tracks, delete_album,
      upload_artwork, remove_artwork, with_album_keys are all raw `actions::run`
      here and albums/favorites need them too.
- [ ] `DownloadsModel::remove` is single-key → "remove all" makes N round trips;
      add `remove_many`.
- [ ] Lift `names.rs` (normalize / joined_credit_primary) into shared code —
      quick_search and the remote artist profile need the same keying. It has 7
      tests; this is the logic that kept regressing in Dioxus.
- [ ] `cx.prompt_for_paths` has no extension filter (rfd had one).

From fullscreen (af979caf):
- [ ] **api::NowPlaying carries no album_id** — the "go to album" link from the
      player can only guess by matching QueueModel::current() on key, which is
      WRONG during a crossfade (the outgoing track is shown). Either add
      album_id to NowPlaying upstream (daemon+proto change) or add
      `store::current_track_info() -> Option<TrackInfo>`. AFFECTS player_bar too.
- [ ] **GPUI has no image blur / backdrop filter** (checked gpui-pre, -base,
      -component: only shadow blur_radius). `cfg.cover_art_blur` cannot be
      honoured as-is; needs a downsampling custom element or a blurred variant
      produced at decode time in `store::artwork`. PARITY gap, tell the user.
- [ ] `store::actions::dont_recommend(key, cx)` — hand-rolled here, player_bar
      needs the identical thing (call, toast, then Next regardless).
- [ ] A shared seek-slider wrapper: SliderState's range is fixed at construction,
      so every seek bar works in 0..1 fractions and re-implements the drag latch
      and the "don't overwrite while dragging" rule. Written 3x in this crate
      alone; player_bar will make it 4.
- [ ] Mute memory lives in view state → lost on drop, not shared with player_bar.
      Belongs on PlayerModel or ConfigModel.
- [ ] `Cover` takes a fixed Pixels size; fullscreen hero wants fill-container.
- [ ] Slider has no underlay slot → no buffered-range underlay on seek bars.
- [ ] fullscreen's queue+lyrics tabs DUPLICATE side_panel; collapse into one
      implementation parameterised by layout once both have landed.

From page_albums (a7e130dc):
- CONFIRMS: components/track_list.rs + store::actions call keys that are missing at
  BUILD time because the fork pins kopuz at 6fe8a59a while the i18n agent added
  them to the working tree. Enable .cargo/config.toml (path patch) at integration
  and verify t() resolves; otherwise raw key names render in every row menu.
- [ ] `TrackList`: no disc/section headers, ROW_HEIGHT private (albums duplicates
      the 60px constant and stacks one list per disc), and `set_rows` cannot
      preserve selection across a rebuild.
- [ ] `Tile` is RenderOnce so `.context_menu` does not apply, and nothing inside
      stops click propagation — a menu slot should own both.
- [ ] `actions::run` drops the answer, so "fetch keys then act" is hand-rolled;
      add a with_album_keys-style helper.
- [ ] Album share (`album_web_url` → `track_web_url` fallback) + artwork
      upload/remove helpers belong in `store::actions` (3 pages need them).
- [ ] en.ftl: no keys for album download toggle, "Disc N" heading, share result.

DONE: theme::apply_config now renders custom themes (Colors::from_vars, dark
inferred from bg luminance). Shell must call apply_config, not apply_named.
NOTE: my mid-flight theme edit briefly broke every crate that links theme (the
sidebar agent saw 41 errors from it). Don't edit shared crates while agents run.

From side_panel (a91606ac):
- [ ] **shell.rs hardcodes the panel slot at w(px(320.))** while the panel drags
      itself 280-600; read `SidePanelView::width()` (exposed for this) or move the
      width into `Chrome` so it persists. MY FILE — fix at integration.
- [ ] TrackDrag drop targets are wired and INERT until TrackList installs a drag
      source (already on the list).
- [ ] `QueueModel`: add total-duration and is-station helpers (fullscreen wants
      the same summary).
- [ ] Note on `Playhead`: 250ms is right for a seek bar, useless for lyrics —
      lyrics drives its own animation frames.
- Karaoke/interlude/depth-blur are approximations: GPUI has no background-clip,
  clip-path, text-shadow or element transform. Fill is per-chunk alpha ramp,
  interlude is a note+progress bar, depth blur is opacity. Tell the user.

From sidebar (a06906296):
- [ ] `SourcesModel` should own connection status + auto-probe on active change;
      sidebar, settings_sources and the player bar would otherwise each probe
      `validate_source` independently. PRIORITISE.
- [ ] No shared `api::Icon` -> element helper (Svg wrapping + FontAwesome
      fallback table). sidebar and settings_sources both need it; belongs in
      components next to `icons`.
- [ ] `ConfigModel::is_locked` key spelling is undocumented, so sidebar left the
      reorder arrows always enabled. Document it or expose typed accessors.
- FIXED A DIOXUS BUG: sidebar_order swap was by display index, which breaks when
  an order key has no item or an unlisted item is appended; now swaps by key.

From quick_search + metadata_dialog (af54a6cb):
- [ ] `Chrome` has no "who had focus" notion: every conditionally-rendered
      overlay must save/restore focus or the Shell key context leaves the
      dispatch path and Space/Ctrl+K/Escape go dead app-wide. Lift the dance out
      of quick_search before the next overlay copies it.
- [ ] `components::dialogs::Dialogs` is a single-field global set with set_global —
      a second registrar silently CLOBBERS the metadata opener. Make it a map.
- [ ] No shared label/value detail-row widget (album/artist headers want it).
- [ ] `theme::overlay` maps to accent_deep, which is opaque, so every scrim calls
      .opacity() itself — add a scrim token. MY mapping choice.
- Note: ↑/↓ must be stolen with capture_action, since Input binds them in the
  Input context and bindings beat key-down listeners (same root cause as space).

From page_playlists (ab9e5d8a):
- [ ] **`TrackActions::reorder` is a DEAD FLAG** in components/track_list.rs: the
      event exists, the page handles it, but the row never renders the up/down
      controls. Render them (or expose a per-row trailing slot). MY BUG.
- [ ] `Tile::on_click` covers the whole tile, so anything in `.overlay()` (dots
      menu, download button) also navigates — playlists wrote a local Card, as
      artists and albums did. THIRD crate to work around this; fix Tile.
- [ ] `PlaylistsModel` only has create/add_tracks; rename/delete/move/folder
      mutations are re-spelled per page. Add store::playlist_actions.
- [ ] `store`: no offline/connectivity signal, so §2.9 offline filtering is out.
- Deviation: deletes now confirm via open_alert_dialog (Dioxus deleted silently).

From page_home (ac7feb70):
- [ ] `Tile` overlay is always visible and declares no hover group → every card
      page wraps it by hand. Add `Tile::hover_overlay(..)`. (4th Tile complaint.)
- [ ] Lift `AlbumPlayButton` into components — §2.1 says "everywhere".
- [ ] Lift the horizontal scroller + row heading into components (discover too).
- [ ] `components::track_list::row_menu` is private and row-shaped; expose a
      `track_menu(track, entries)` builder (Home song cards, Discover SongCard,
      fullscreen actions menu all duplicate it).
- [ ] `store::actions::set_favorite_many(keys, on)` + aggregate failure toast.
- Home shuffles with a per-page seed: stable while open, not per frame. Correct.

UNVERIFIED (seen once, not reproduced):
- [ ] With nothing playing, the player bar's seek track rendered fully green
      rather than empty. `played_fraction` returns 0 for a None duration and the
      slider is built min 0 / max 1 / default 0, so the cause is not obvious;
      it may be the disabled-slider rendering or the theme's slider token. Run
      the app, look at the bar idle, and settle it before touching the code.

OPEN QUESTIONS FOR THE USER:
- The fork pins kopuz at 6fe8a59a. The 20 new i18n keys live in the upstream
  WORKING TREE only, so the app reads them only through .cargo/config.toml
  (the local path patch, gitignored). A fresh clone renders raw key names until
  those keys are committed upstream and the pin is bumped. Upstream locale CI
  will also fail until they are translated into the other 27 .ftl files.
- api::NowPlaying has no album_id: the player's album link guesses via the queue
  and is wrong during a crossfade. Daemon-side fix or a store resolver.

MOTION PASS (after all motion agents land):
- [ ] Widen `components::motion` to take `impl Into<TransitionId>` instead of
      `impl Into<ElementId>`. `TransitionId` already has
      `From<(I: Into<ElementId>, C: Into<SharedString>)>`, which is the
      "id + channel" shape every call site wants, and three crates have now
      hand-rolled the same `channel()` helper to work around the narrow bound.
      Deferred because it is a signature change and five agents were mid-flight.
- [ ] Lift `motion::appear` (wrapping `base::motion::Presence`) and
      `motion::reveal` (wrapping `MotionReveal`) into `components::motion`:
      overlays, quick_search and metadata_dialog all reach past the facade into
      `gpui_kit::base::motion` for entrance fades and the collapse reveal.
- [ ] A hover helper for view-less render paths (`RenderOnce`, `Popover::content`
      have no entity for `cx.listener`); switcher.rs hand-rolls
      `use_keyed_state` + a track() closure factory.
- [ ] `motion::INSTANT` (Duration::ZERO) — load-bearing in the sidebar, where a
      drag must track the pointer exactly while the collapse snap tweens.
- [ ] A tweenable slider fill: nothing in components can tween a Slider's bar, so
      the seek and volume fills sit outside the motion system.
- [ ] **`motion::tweened(button, fill, text, edge, cx)`** — asked for by three
      agents independently, and each wrote a private copy. A Button's own hover
      refinement paints over an instance `.bg()`, so a naive `.bg(tween)` snaps
      on hover-in and only tweens on the way out; the fix is a
      ButtonCustomVariant whose `hover`/`active` get the SAME tweened colour.
      Without it half of `transition-colors` is silently lost on every button.
- [ ] **`motion::hover_flag(id, window, cx)`** + its listener — five private
      copies now exist. Must be built on `window.use_keyed_state`, since a
      `RenderOnce` (Card, Tile, SettingRow) has no `cx.listener`.
- [ ] `motion::SNAP` (120ms) and `SLOWER` (500ms): the settings rows are 120ms
      and several covers are `duration-500`; feature crates import
      `std::time::Duration` to say so.
- [ ] A zoom clip frame helper — `div().size(base).rounded(r).overflow_hidden()
      .flex().items_center().justify_center()` is repeated at ~6 sites.
- [ ] Press feedback: `active:scale-95` is on nearly every Kopuz button and has
      no story at all. A one-frame wash darken would cover it uniformly.
