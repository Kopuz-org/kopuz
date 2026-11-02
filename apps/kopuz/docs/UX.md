# What is wrong with this UI, and the order to fix it

An audit of the port against ordinary desktop expectations, not against the
original: where the Dioxus app is also wrong, that is noted and fixed here
anyway. Severities are the auditors'. Tick items as they land.

## Broken — the feature does not work at all

- [x] **Compact mode drops the toast, dialog and disconnect layers.**
      `app/src/shell.rs:203` returns early with only the mini player, above both
      the `Root` layers and the `connected` check. In mini-player mode no toast
      can appear, no dialog can open, and a daemon disconnect shows nothing —
      the last track stays on screen forever.
- [x] **A failed daemon read renders as "there is nothing here."**
      `store/src/query.rs:390` keeps the error but only two call sites ever read
      it. Everywhere else is `value().unwrap_or_default()` + `is_settled()`, so
      a refused call shows "No tracks in your library" / "Album not found".
- [ ] **The app quits with no window if the link cannot be set up.**
      `app/src/main.rs:56` calls `cx.quit()` before a window exists, so from a
      desktop icon it appears to do nothing. `Connection` already models
      `Unreachable` and would retry.
- [x] **Nothing can be dragged to the queue.** `TrackDrag` is declared and the
      queue registers `on_drop` + a drop-target highlight, but no row ever calls
      `.on_drag` — `components/src/track_list.rs:117`, `side_panel/src/queue.rs:83`.
      A port regression; the original drags rows.
- [x] **Playlist reorder is dead end to end.** The page computes `reorder`, the
      list declares `TrackListEvent::Reorder`, the handler calls the API — and
      `row_menu` never reads the flag, so nothing emits it.
      `components/src/track_list.rs:100`, `page_playlists/src/detail.rs:173`.

## Bad — the feature works but misleads or blocks

- [x] **Selection sticks at "0 selected" and rows stop playing.** `set_rows`
      drops vanished keys but never re-evaluates `selecting`
      (`components/src/track_list.rs:171`), so re-sorting with select-all on leaves
      the pill over a spinner and clicks toggle checkboxes instead of playing.
- [x] **Multi-select is unreachable on Search, Genre and Playlist detail**, and
      there is no ctrl-click or shift-click anywhere — ranges must be ticked one
      checkbox at a time. `components/src/track_list.rs:312`.
- [x] **No keyboard navigation in any list.** Nothing but the shell holds a
      focus handle. No arrows, no Enter, no ctrl-a; Escape does not clear a
      selection. Quick-search already does this properly and is the model.
- [ ] **Library's column header looks sortable and is inert.** Same
      `columns::frame()` as Favorites and Artist detail, which both sort on
      click. `page_library/src/lib.rs:357`.
- [x] **Every error reads `DebugEnumVariant: raw message`.** ~20 sites do
      `Notices::error(error.to_string())`. `page_discover/src/shelves.rs:30` is
      the one place that maps a code to human text; that should be the default.
- [x] **The connection screen claims failure before anything failed** and never
      says it is retrying: a cold start reads "Can't reach the kopuz daemon /
      ● Connecting…". `app/src/shell.rs:147`.
- [x] **"Start Daemon" has no visible effect and swallows its failure**, so it
      gets clicked repeatedly, spawning a process each time. `app/src/shell.rs:165`.
- [x] **A dismissed playback error stays dismissed forever**, keyed on the
      message string — the same failure never speaks again.
      `overlays/src/lib.rs:54`.
- [ ] **Player and queue edits are fire-and-forget with no rollback.** A failed
      command leaves the play glyph flipped and the playhead ticking; a failed
      queue move leaves the row where it was dropped while the real order
      differs. `store/src/player.rs:259`, `store/src/queue.rs:204`.
- [x] **A download batch finishing is never acknowledged** and the failed-track
      list is destroyed with the card. The queue now lives in `DownloadsModel`,
      outlives the job, and ends with a summary toast that opens the popover.

## Rough — works, feels unfinished

- [x] Re-sorting or typing blanks the page to a spinner: `set_fetch` now keeps
      the held value and marks it `is_stale()`. Skeletons themselves are §4.
- [x] The artists grid builds every tile; so do the playlists grid, search album
      results and artist-detail albums. `page_artists/src/grid.rs:183`.
- [x] The albums grid re-clones, re-dedupes and re-sorts the whole library on
      every hover, because hover calls `cx.notify()`. `page_albums/src/grid.rs:320`.
- [ ] Empty states name the emptiness and stop. Library's is a bare italic line,
      not even the `EmptyState` widget. Favorites is the counter-example.
- [x] Long titles truncate with no tooltip, anywhere.
- [ ] Playlist detail has no loading state, no empty state, no column header.
- [x] Cancelling a job has no pending state and swallows its failure.
- [x] The scan banner cannot be cancelled, unlike the download card.
- [x] Browser sign-in shows nothing while the browser is open — click, then
      nothing, for as long as it takes. Now a spinner and "Waiting for your
      browser…", with a toast on success.
- [x] If the config read fails every settings control renders enabled and dead.
      The page says so instead, and swaps in when the daemon answers.
- [x] The sidebar status dot latches at its first probe and then lies.
- [x] Error toasts auto-dismiss in 5s and never dedupe.
- [ ] Every page fetches the entire library; paging is a TODO.

## Polish

- [ ] Albums sort button never shows direction; artist-detail headers give no
      hint they are clickable.
- [ ] Nothing in a grid marks what is currently playing.
- [x] A cover that fails to load shows a blank square, not the placeholder.
- [ ] A daemon notice with no `en.ftl` entry renders its raw snake_case key.

## Confirmed good, leave alone

`FavoritesModel::set` is the model optimistic update — applies, reverts the
exact flip, toasts. `Query`'s generation guard and `ConfigModel`'s edit counter
both stop a stale answer clobbering a newer edit. The reconnect loop is sound;
only the messaging about it is thin. Favorites is the model page: tri-state
column sort, debounced filter, three distinct actionable empty states. The
now-playing row treatment is clear. The row's dots menu and context menu are
built from one closure, so they cannot drift.

## Also fixed, found while fixing the above

- Equalizer previews were a daemon call per pointer move; throttled, with the
  release always sent so a drag never ends on a stale curve.
- The artwork settings form rebuilt its inputs on every settled read, so tabbing
  between fields swapped the next one out from under the caret.
- The lyrics panel asked for an animation frame whenever a synced track played,
  re-rendering the whole window at refresh rate rather than at the playhead's
  four ticks a second.
- The queue had no way to remove a row, though `QueueModel::remove` existed.
- A non-numeric track or disc number was read as "clear this tag".
- Re-sign-in offered a password box for a service whose sign-in is a browser,
  because `sign_in` reads `None` once a source holds any credentials. The daemon
  gained `SourceInfo.reauth`, which answers for the service rather than the
  current state.
