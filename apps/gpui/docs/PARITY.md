# Kopuz GPUI port: feature-parity checklist

Source of truth: Dioxus frontend at kopuz `master` (6fe8a59a). Crates read: `pages`, `components`, `hooks`, `kopuz_route`, plus `kopuz/src/main.rs` (the App shell lives there, not in `pages`).
Notation: `api.x()` = `KopuzApi` method; `ev:X` = `ApiEvent::X`; `inv:T` = `ev:LibraryInvalidated{table:T}` re-runs the read; `caps.x` = `SourceCapabilities.x` of the active `SourceInfo`; `cfg.x` = `config::Settings` field (persisted as a typed `SettingsEdit` through the debounced `api.edit_config`, see §5).
Shared verbs used below:
- **PLAY_LIST(rows, i)** = `api.set_queue(SetQueueRequest{Replace, Tracks{keys}, start_index:i})`
- **PLAY_ALL(rows)** = if shuffle on: PLAY_LIST at a random start index (shuffle flag untouched), else PLAY_LIST(rows, None)
- **APPEND(rows)** = `set_queue{Append, Tracks{keys}}`; **PLAY_NEXT(rows)** = `set_queue{PlayNext, Tracks{keys}}`
- **ADD_TO_PL(keys)** = PlaylistModal → `api.add_playlist_tracks(id, keys)` or `api.create_playlist(name, keys)`
- **DELETE(keys, disk)** = `api.delete_tracks(keys, from_disk)`
- **DL(keys)** = `api.download(keys)`; **UNDL(keys)** = `api.remove_download(key)` per key
- **RADIO_T(key)** = toast "radio_starting" → `set_queue{Replace, TrackRadio{key}, start_index:0}`, toast_error "radio_failed" on error
- **RADIO_P(id)** = same with `PlaylistRadio{id}`
- **SHARE(track)** = `api.track_web_url(key)` → clipboard + toast "Copied link" / "Couldn't find a page for this track"
- **JOB(kind)** = `api.start_job(kind)`, `ErrorCode::Conflict` swallowed, other errors toast
- Every mutation helper toasts `error.to_string()` on failure (hooks `*_actions.rs`).

---

## 1. Routes

`Route` enum (`kopuz_route`), no URL params: params live in shell signals and are captured in `NavSnapshot` for back-navigation.

| Route | "Params" (shell signals) | Reached from |
|---|---|---|
| Home | none | sidebar (default at launch) |
| Search | `search_query: String` | sidebar; Discover/DiscoverArtist "search artist" sets query + navigates |
| Discover | none | sidebar (hidden unless `caps.discover`) |
| DiscoverPlaylist | `discover_playlist_id`, `discover_playlist_title` | Discover/DiscoverArtist playlist tile, song-list shelf "Show all" (`more_ref`) |
| Library | none | sidebar |
| Album | `selected_album_id` ("" = grid, else detail) | sidebar ("albums", clears id), any album card/track-row album link/bottombar title/fullscreen album link, Home cards, Search album hits, Discover album tiles, artist album cards |
| Artist | `selected_artist_name`, `selected_artist_channel_id: Option` ("" + None = grid) | sidebar ("artists", clears both), track-row artist link, album-detail artist link, bottombar artist, fullscreen artist, Home Top Artists, Discover artist tile (sets channel id) |
| Playlists | `selected_playlist_id: Option` (None = grid) | sidebar, Home playlist card |
| Favorites | none | sidebar |
| Activity | none | sidebar |
| Radio | none | sidebar |
| Downloader | none | sidebar (desktop only, `cfg(not android)`) |
| Settings | internal `SettingsCategory`; shell `SettingsAnchor: Option<element-id>` | sidebar bottom item; source switcher "Manage sources" (anchor `settings-media-servers` → Library category) |
| ThemeEditor | none | not in sidebar; editor is embedded in Settings → Appearance (`embedded: true`); standalone route kept (desktop only) |

- [ ] Route dispatch for `Artist`: if `caps.artists == ArtistPresentation::Remote` AND a selection exists → `DiscoverArtistPage`; else library `Artist` page
- [ ] Album detail falls back to `api.catalog_detail(Album, id)` when `api.album(id)` is None (catalog/search-opened albums)
- [ ] Back navigation: `NavigationController.history: Vec<NavSnapshot>` pushed on every change of (route, album_id, artist_name, artist_channel_id, playlist_id, discover_playlist_id/title); `go_back()` pops + sets `restoring` so the restore is not re-pushed
- [ ] `navigate_to_artist(name)` clears channel id; `navigate_to_album(id)`; `close_playlist()` (clears selection with restoring)
- [ ] Sidebar click on Album/Artist resets the detail selection to the grid
- [ ] Source switch (active server id changes) clears `selected_playlist_id` and calls `reset_for_backend_switch` (Stop + empty Replace queue); Playlists page also clears selection when `cfg.active_source` changes

---

## 2. Pages

### 2.1 Home (`pages/home.rs`, `home_body.rs`, `home_body_sections.rs`)
Data: `api.albums(all)` inv:Albums · `api.albums_recently_added({0,64})` inv:Albums+Tracks · `api.artists(all)` inv:Tracks · `api.playlists()` inv:Playlists+Folders · `api.recent_tracks({0,50})` inv:Recents · `api.top_genre()` inv:Tracks · `api.artist_sample_tracks({0,30})` inv:Tracks · `api.tracks_by_keys(offline keys)` · hero `api.album_tracks(id, all)` · `api.favorites()` inv:Favorites
- [ ] Header "home" title (hidden in Vaxry), "Customize home" button → edit mode
- [ ] Edit mode: per-section row with label, move up/down (swap in `cfg.home_sections`), show/hide toggle (`enabled`), Listen Now layout toggle (`cfg.listen_now_style` List⇄Cards); "Reset home" → `config::default_home_sections()`; "Done"
- [ ] Sections rendered in `cfg.home_sections` order, disabled ones hidden (shown at 40% opacity in edit mode). Keys: `hero, continue_listening, listen_now, top_artists, new_releases, made_for_you, recently_added, playlists`
- [ ] Hero ("featured_album"): newest recent track; cover = album Full art else track Full art; gradient overlay; title + "by_artist_full"; "Start listening" → `set_queue{Replace, Album{id}, start_index:0}`; heart = all album tracks favorited → `api.set_favorite(key,on)` for each track not already in target state (reads `api.favorites()` first), toast "Couldn't update some favorites" on partial failure
- [ ] Hero height resizable by drag handle in edit mode (140–800 px) → `cfg.hero_height`
- [ ] Hero empty state when no recents (blank block)
- [ ] Continue Listening: up to 10 recent tracks, one per album, skipping unknown artist/album; horizontal scroller with ‹ › buttons (scrollBy ±300); card click → album; hover play button (AlbumPlayButton); card dots menu: Play next (PLAY_NEXT), Add to queue (APPEND), Start radio (RADIO_T, `caps.track_radio`), Share (SHARE)
- [ ] Listen Now: shuffled albums (dedup by lowercased title, drop unknown artist/album), skip first; Cards (10) or List grid (8); click → album; play button
- [ ] AlbumPlayButton semantics (everywhere): shows pause if current track's `album_id` == card id and playing; click toggles (`player_command(Toggle)`) when current, else `set_queue{Replace, Album{id}, 0}`
- [ ] Top Artists: up to 10 distinct artists from `artist_sample_tracks` (or offline tracks), photo from `ArtistInfo.artwork`; click → Artist page (by name)
- [ ] New Releases: 12 albums sorted by year desc, dedup title
- [ ] Made For You: 12 random albums whose `genre == top_genre`; title uses genre
- [ ] Recently Added: 12 from `albums_recently_added`, dedup title
- [ ] Playlists: last 10 playlists (reverse order), cover from `PlaylistInfo.artwork`, "music_playlist_count"; click → playlist detail; dots menu with only Start radio (RADIO_P) when `caps.playlist_radio`
- [ ] Offline mode (`caps.downloads && is_offline`): albums/artists/playlists filtered to fully-downloaded content (see §6 seam note)
- [ ] Auto-sync: if `caps.sync` and albums empty on first load → JOB(LibrarySync)
- [ ] Each empty section renders nothing

### 2.2 Search (`pages/search.rs`, `components/search/*`)
Data: `api.search(query)` (lowercased, re-run on inv:Tracks/Albums; `None` when blank) · genres derived client-side from `api.albums(all)` (split `genre` on `/;,`, first album cover per genre) · `api.genre_tracks(genre, all)` inv:Tracks
- [ ] Search bar, 150 ms debounce, keydown stops propagation (so Space doesn't toggle playback)
- [ ] Blank query → "Browse genres" grid (genre tiles with cover); empty → "no_genres_found"
- [ ] Results: Tracks list (virtualized TrackRow, full track-row menu, play = PLAY_LIST(results, i)); Albums grid (click → Album). Layout differs if only albums. "no_results_found{query}"
- [ ] Genre detail: back, genre title, track count, Play (PLAY_ALL), Shuffle (toggle shuffle + play shuffled), virtualized TrackRow list with column sort
- [ ] Track-row delete in Search/Genre detail always `DELETE([key], true)` (not caps-gated; parity bug to decide)
- [ ] Add to playlist via PlaylistModal (ADD_TO_PL([key]))

### 2.3 Discover (`pages/server/discover.rs`, only when `caps.discover`)
Data: `api.catalog(continuation)` · `api.catalog_detail(CatalogDetailRequest)`
- [ ] Unsupported state "discover_unsupported" when `!caps.discover`
- [ ] Initial spinner; error text (`ErrorCode::SourceAuthExpired` → "source_anon_discover", else "discover_failed{error}")
- [ ] Shelves (title, optional strapline), horizontal scroll with ‹ › (±800 px)
- [ ] Infinite scroll: sentinel + IntersectionObserver (600 px margin) → `api.catalog(Some(token))`, "discover_more_loading"
- [ ] Tile kinds: Album/Playlist card (click → Album route / DiscoverPlaylist), Artist card (round; click → DiscoverArtistPage with channel id), Mood/Unknown (inert), Track = SongCard
- [ ] Card hover play button: `play_catalog(kind,id)` pages `catalog_detail` until `continuation` is None; first page → `set_queue{Replace, Tracks}`, rest `Append`; dedup keys; sets `browse_loading`; caches full list only when complete
- [ ] Hover prefetch after 250 ms dwell (disarmed on mouseleave) into `DiscoverPrefetchCache` (app-wide), used by play_catalog
- [ ] `DiscoverNowPlaying` (app-wide): tile whose id started playback shows pause/loading; click toggles instead of refetching
- [ ] SongCard click: RADIO_T(key) if `caps.track_radio`, else `set_queue{Replace, Tracks{[key]}}`
- [ ] List shelves (`shelf.list`): TrackRow list, play = `set_queue{Replace, Tracks{keys}, start_index:i}`, "discover_show_all" → DiscoverPlaylist(more_ref, title)
- [ ] Failure while appending pages sets `playback_error` banner

### 2.4 DiscoverPlaylist (`DiscoverPlaylistDetail`)
- [ ] Load `catalog_detail{kind: Album if id starts "MPRE" else Playlist}` (service-specific id sniff, see §6); generation guard drops stale answers
- [ ] Loading spinner / error / "playlist_not_found"; loaded → TrackListView (read-only header, full track list features, no delete/reorder)

### 2.5 DiscoverArtistPage (remote artist profile)
- [ ] `catalog_detail{Artist, id = channel id or name}`; generation guard; "artist_none_selected"
- [ ] Banner (artwork bg), title, subtitle, description (3-line clamp), Shuffle button → `play_catalog(Playlist, playback_id)`
- [ ] Shelves rendered with the same ShelfRow/tiles as Discover; artist tiles re-open this page; back button

### 2.6 Library (`pages/library.rs`)
Data: windowed `api.tracks(TrackFilter{sort: Fields(cfg.library_sort)}, Page{offset,limit})` + count query `{0,1}` for total, both inv:Tracks · `api.albums/artists/playlists` for stat cards · `api.downloads()` + JobProgress(Download)
- [ ] Title "your_library"; refresh button: `caps.scan_folders` → trigger local rescan (shell), else `caps.sync` → JOB(LibrarySync)
- [ ] Auto-sync once when `caps.sync` and total == 0
- [ ] Stat cards: tracks (total), albums (distinct lowercased titles), artists, playlists
- [ ] Select-all checkbox (fetches all rows via `api.tracks(filter, all)`, selects keys)
- [ ] SortControl (stacked criteria) over Title/Artist/Album/Duration/DateAdded → `cfg.library_sort`
- [ ] Virtualized list (60 px rows, 10-row buffer), scroll persisted per route; column header
- [ ] Row click play: fetch whole filtered list `api.tracks(filter, all)` → PLAY_LIST(all, idx)
- [ ] Now-playing row highlight (only when queue length == total)
- [ ] Track row menu (§3.9) with: metadata edit (`caps.edit_tags`), delete (`DELETE([key], caps.delete_from_disk)`, hidden unless `caps.delete_from_disk`), download toggle (`caps.downloads`), radio
- [ ] Multi-select (long-press / checkbox) → SelectionBar: Add to queue (fetch all, filter selected, APPEND), Add to playlist (ADD_TO_PL), Delete (if `caps.delete_from_disk`), Cancel
- [ ] States: spinner while total unknown or syncing; "no_tracks_found"; trailing spinner while syncing

### 2.7 Albums (`pages/album.rs`)
Grid data: `api.albums(all)` inv:Albums; detail: `api.album(id)` inv:Albums, `api.album_tracks(id, all)` for every same-titled album inv:Tracks, `api.catalog_detail(Album,id)` when `caps.albums == Remote` (online) or DB miss
- [ ] Grid title "all_albums"; auto JOB(LibrarySync) once if empty and `caps.sync`
- [ ] ViewModeToggle Grid/List → `cfg.album_view_mode`; SortControl over available fields (Title, Artist, +Year if any year, +Genre if any genre) → `cfg.album_sort`; client-side sort
- [ ] Grid dedups by lowercased title; offline filter to downloaded albums
- [ ] Card: click → detail; right-click (contextmenu) or dots → menu: Add all to queue (`album_tracks` sorted → APPEND), Add all to playlist (ADD_TO_PL(album keys)), Delete album (`caps.delete_from_disk` → `api.delete_album(id, true)`) / Remove from cache (else `delete_album(id,false)` for every same-titled album)
- [ ] Grid scroll persisted (`scroll_persist` key "albums"), restored once after first render
- [ ] "no_albums_found"
- [ ] Detail (Standard): TrackListView with album name, artist (click → artist), cover, year, disc grouping, "album_not_found"
- [ ] Detail cover click (`caps.edit_tags`): native file picker (jpg/png/webp) → `api.upload_artwork{Album(id), content_type, bytes}`; trash button (if artwork) → `api.remove_artwork(Album(id))`
- [ ] Detail: per-track delete + selection delete (`caps.delete_from_disk`, from_disk=true); per-track download toggle, Download all / Remove all (`caps.downloads`); download-all disabled while any track active
- [ ] Detail (Remote, `caps.albums == Remote`) "YtAlbumDetail": left meta column (cover, artist link, title, "Album • N songs • M min • year"), buttons: download-all/remove-all toggle, go to artist, Play (PLAY_ALL), Shuffle toggle, Share (`api.album_web_url(browse_id)` fallback `api.track_web_url(first key)` → clipboard); track list of TrackRows (radio, queue, playlist, download toggle; no delete)
- [ ] Offline: detail tracks filtered to downloaded

### 2.8 Artists (`pages/artist.rs`, library presentation)
Data: `api.albums(all)`, `api.artists(all)` (counts + photos), `api.artist_sample_tracks({0,MAX})`, `api.artist_tracks(name, all)`; `api.refresh_artist_artwork(names)` fired when the grid's name set changes (results arrive as inv:Tracks)
- [ ] Grid: artist set = album artists ∪ sample-track `artists`, normalized, dropping joined collab credits whose primary exists; photo = `ArtistInfo.artwork`; placeholder mic icon
- [ ] ViewModeToggle → `cfg.artists_view_mode`; SortControl Name/Tracks/Albums → `cfg.artist_sort` (name breaks ties)
- [ ] Grid scroll persisted (key "artists")
- [ ] Detail: back button; Tracks⇄Albums toggle → `cfg.artist_view_order`
- [ ] Detail Tracks (Showcase): header w/ photo (click → pick file → `upload_artwork{Artist(name)}`), Play all, shuffle, download all/remove all, select all, sortable columns, full row menu, metadata edit, selection bar (queue / playlist / delete)
- [ ] Detail Albums: ViewModeToggle → `cfg.artist_album_view_mode`, SortControl (no Artist field) → `cfg.artist_album_sort`; card menu: Add all to queue, Add all to playlist (`caps.playlists != None`), Delete album (`caps.delete_from_disk`), Download/Remove downloads (`caps.downloads`, uses `with_album_keys`)
- [ ] States: "no_tracks_found", "no_albums_found"; offline filter

### 2.9 Playlists (`pages/playlists.rs`, `components/playlist/*`)
Data: `api.playlists()` → `PlaylistCatalog` inv:Playlists+Folders; detail tracks `api.tracks_by_keys(track_keys)` inv:Tracks
- [ ] Header: New folder (`caps.folders` → `api.create_playlist_folder(t("new_folder"))`), Add playlist (popup)
- [ ] AddPlaylistPopup: name input, Save → `api.create_playlist(name, [])` (refused with "error_server_not_configured" if `caps.playlists == None`), inline error; "Add folder" (desktop, `caps.folders`): pick directory → `api.folder_tracks(prefix, all)` → `create_playlist(dirname, keys)`
- [ ] Folder layout (`caps.folders`): folders grid (cover = first playlist's), open folder view with breadcrumb/back, root playlists grid under "playlists" heading; empty "no_playlists_yet"
- [ ] Folder card menu: Rename (RenameTextModal, Enter saves → `api.rename_playlist_folder`), Delete (`api.delete_playlist_folder`)
- [ ] Playlist card menu: Move to folder (FolderPickerModal: pick folder → `api.move_playlist(pid, Some(fid))`; "new folder" inline create → `create_playlist_folder` then `move_playlist`), Remove from folder (`move_playlist(pid, None)`, only inside a folder), Rename (`api.rename_playlist`), Start radio (RADIO_P, `caps.playlist_radio`), Delete (`api.delete_playlist`)
- [ ] Flat remote layout (`!caps.folders`): JOB(PlaylistSync) on mount when `caps.sync`; if `caps.albums == Remote` sync status row ("Loading tracks — done/total", "N playlists synced", Refresh → JOB(PlaylistSync)) from JobProgress(PlaylistSync); card with "Server • N tracks", radio dots menu, hover download/remove-downloads button (`caps.downloads`, spinner while any key active)
- [ ] Empty: anonymous source → "source_anon_playlists" else "no_playlists_found"
- [ ] PlaylistDetail: on mount `api.refresh_playlist(id)` when `caps.sync`; TrackListView with Start radio (RADIO_P), cover click → `upload_artwork{Playlist(id)}`, metadata (`caps.edit_tags`), row "Remove from playlist" → `api.remove_playlist_track(id, index)`, reorder ▲▼ buttons when `caps.playlists == Reorder` → `api.reorder_playlist(id, {from,to})`, delete track (`DELETE([key], caps.delete_from_disk)`), download all/one/remove (`caps.downloads`), "playlist_not_found"

### 2.10 Favorites (`pages/favorites.rs`, `favorites_body.rs`)
Data: `api.favorites()` inv:Favorites → `api.tracks_by_keys(refs)` inv:Tracks; JobProgress(FavoritesSync)
- [ ] Title, SearchBar filtering client-side (title/artist/album/artists), scroll resets on query change
- [ ] Column-header sort (Title/Artist/Album/Duration, tri-state asc/desc/none)
- [ ] Select-all, selection bar (queue / playlist / delete)
- [ ] Virtualized rows (60 px), play = PLAY_LIST(sorted filtered list, i); full row menu incl. radio, download, metadata, delete
- [ ] Sync UI: `caps.favorites_sync == Instant` + running → "syncing_with_server" spinner; `Paginated` → status row ("yt_syncing_progress{count}" / "yt_synced_total{count}") + Refresh → JOB(FavoritesSync)
- [ ] Empty states: loading spinner; "no_results_found{query}"; anonymous source → "source_anon_favorites"; else "no_favorites" + "heart_track_to_add"

### 2.11 Activity (`pages/activity.rs`)
Data: windowed `api.tracks(TrackFilter{sort: PlayCount})` + count; genre per album from `api.albums(all)`; play counts read from `cfg.listen_counts[source.listen_count_key(uid)]` (not `api.stats()`, see §6)
- [ ] "listening_logs" / "most_played_tracks" header; columns #, title (cover+artist), album, genre, time, plays (fire icon if >0)
- [ ] Row click → fetch all with same filter → PLAY_LIST(all, idx); virtualized; scroll persisted
- [ ] Spinner / "no_tracks_in_library"

### 2.12 Radio (`pages/radio.rs`)
Data: `api.radio_stations()` (refetched after pin) · `api.search_radio(query, 60)` (empty query = top stations)
- [ ] Filter input (300 ms debounce) filters pinned list client-side AND drives directory search
- [ ] Selected (pinned) stations table: station/description/streams columns; row click → `set_queue{Replace, Radio{station_id, first stream}, 0}`; per-stream buttons; overflow "+N" expander per station
- [ ] Directory list: "radio_search_results" / "radio_top_stations", "radio_powered_by"; loading "radio_loading_stations"; error "radio_search_failed"; empty "radio_no_stations_match"; play; pin toggle
- [ ] Pin/unpin: optimistic local set, `api.pin_radio_station(id, !pinned)`, revert + toast on failure
- [ ] Station/stream names and descriptions may be i18n keys → run through `i18n::t`
- [ ] Artwork via `RadioStationInfo.artwork` (`ArtworkTarget::Station`)

### 2.13 Downloader (desktop only, `pages/downloader.rs`)
Data: `api.download_formats()`, `api.downloader_settings()`, `api.downloader_history()`, JobProgress(UrlDownload)
- [ ] URL input (Enter or button) → `api.download_url(url, format)`; refusal shown inline (red box)
- [ ] Format chips from `download_formats` (first = default)
- [ ] Options toggle → SchemaForm over `downloader_settings` → `api.set_downloader_settings([value])`
- [ ] Active row: phase "processing"/"downloading {percent}"/waiting, title = job message or shortened URL, progress bar
- [ ] History rows (finished/failed + error, format label); "Clear history" → `api.clear_downloader_history()`; history reloads when job stops running
- [ ] Empty state "downloader_empty_state"

### 2.14 Settings (see §4 for fields)
- [ ] Category navigation "fan" (8 categories; Tools hidden on Android); `cfg.settings_layout` Cd (fan) vs TopBar
- [ ] Managed-settings notice per lock kind when `ConfigView.locks` is non-empty; locked rows show lock + disabled control, citing the pinning file or env var
- [ ] Build info card (version + commit, copy to clipboard)

### 2.15 Theme editor (desktop; embedded in Settings → Appearance)
- [ ] Saved custom themes list (sorted by name), "+ New theme"
- [ ] Name input; 13 color rows (color picker + hex text input with sanitize/normalize, 3→6 digit expansion, revert on blur)
- [ ] Live preview card (title, time, progress bar, accent/highlight/danger chips)
- [ ] Save → `cfg.custom_themes[id] = CustomTheme{name, vars}` (id `custom-<slug>[-n]`); Delete (resets `cfg.theme` to "default" if active)

---

## 3. Global chrome

### 3.1 App shell / window (`kopuz/src/main.rs`)
- [ ] Window 1350×800, title "Kopuz", app icon; decorations = `cfg.titlebar_mode == System` (Linux/Windows), updated live
- [ ] macOS: hidden title, transparent titlebar, fullsize content; 40 px drag region atop sidebar; app menu (Window: fullscreen/hide/hide others/show all/maximize/close/quit; Edit: undo/redo/cut/copy/paste/select all)
- [ ] Custom titlebar (`TitlebarMode::Custom`, Linux/Windows): drag area, "Kopuz", minimize/maximize-toggle/close; Windows WM_NCHITTEST subclass for snap
- [ ] Linux resize handles (8 edges/corners) when not System titlebar and not maximized
- [ ] Top-of-content banners: scan progress (local source, "Scanning library…"/current file, from ev:JobProgress{Scan}/JobFinished{Scan}); playback error (`PlayerState.error.message`, dismissable); offline banner (connectivity probe, "Keep server mode" switches back); update banner (GitHub latest release check if `cfg.auto_check_updates`, "View release" opens browser)
- [ ] Player bar placed Top or Bottom per `cfg.player_bar_position`
- [ ] RTL support: `dir=rtl` from `i18n::is_rtl()`; transport/sliders forced LTR; sidebar border/resize handle mirrored
- [ ] Global image error fallback → default cover SVG
- [ ] Fade-in after stylesheets load (cosmetic)

### 3.2 Global keybindings
- [ ] Space → play/pause (ignored in inputs/textareas/selects/contentEditable, with modifiers, IME composing, or key repeat)
- [ ] Escape → exit fullscreen player; exit compact mode
- [ ] Ctrl/Cmd+M → toggle compact mini-player
- [ ] Ctrl/Cmd+K → toggle Quick Search
- [ ] Text inputs stop keydown propagation so these don't fire while typing
- [ ] No media-key handling in the frontend: MPRIS/SMTC/macOS Now Playing are driven by the daemon (desktop)

### 3.3 Sidebar (`layout/{normal,vaxry}/sidebar.rs`)
- [ ] Normal style: flat list Home, Search, Discover, Library, Albums, Artists, Playlists, Favorites, Radio, Activity, Downloader (desktop) + separator + Settings
- [ ] Normal: order from `cfg.sidebar_order`, hover ▲▼ reorder buttons per item (writes `cfg.sidebar_order`)
- [ ] Vaxry style: sections "discover" (Home, Search, Discover, Radio) and "library" (Library, Albums, Artists, Favorites, Activity, Playlists), then Downloader + Settings; no reorder
- [ ] Discover item hidden unless `caps.discover`
- [ ] Drag-to-resize width (Normal default 240, Vaxry 200; collapse to 72 px icon rail under 150 px, expand over 180; max 450); collapsed items show tooltips
- [ ] Active item indicator bar
- [ ] Source switcher at top when `cfg.show_source_toggle` (§3.4)

### 3.4 Source switcher (`navigation/source_switcher.rs`)
Data: `api.sources()` inv:Servers; `api.validate_source(active.id)` on every active change (server only)
- [ ] Trigger: active source glyph (`ServiceRef.icon`, `Icon::Class`/`Icon::Svg`), name, status dot (Online green / Connecting pulsing amber / Offline red), chevron; brand accent via `ServiceRef.accent` (local = theme indigo)
- [ ] Popover: "sources" + count, rows (name + subline = service name uppercased or "source_on_this_device"), active check; click → `api.switch_source(id)` then `api.config()` pulled into `cfg`; errors toast
- [ ] Collapsed-rail variant (icon only, popover to the side)
- [ ] Footer "Manage sources" → Settings (Library category, scroll to media servers)

### 3.5 Player bar (`layout/{normal,vaxry}/bottombar.rs`, `playback/controls.rs`)
Mirrors PlayerController signals (§5.1)
- [ ] Cover thumb, title (click → album of current track), artist (click → artist)
- [ ] Heart: `use_track_is_favorite` (reads `api.favorites()`, inv:Favorites) → toggle: `api.favorites()` then `api.set_favorite(key, !current)`; toast on refusal
- [ ] Don't-recommend button (only `caps.dont_recommend`; disabled while favorited) → `api.dont_recommend(key)` then `PlayerCommand::Next` regardless; toast on error
- [ ] Transport: shuffle toggle (`SetMode{shuffle}`), prev (`Previous`), play/pause (`Play`/`Pause`, optimistic), next (`Next`), repeat cycle None→Queue→Track (`SetMode{loop_mode}`, "1" badge for Track); titles shuffle_on/off, repeat_off/queue/track
- [ ] Seek slider: time labels (blank length for radio), drag shows local value, commit → `Seek{position_ms}` (optimistic anchor); disabled for radio (duration u64::MAX) or loading; buffered ranges underlay (merge gaps ≤0.75 %, snap to playhead within 5 %); indeterminate scan bar while loading with no ranges
- [ ] Volume: mute toggle (remembers pre-mute level), slider (`SetVolume` live on input), wheel step = `cfg.volume_scroll_step`; persisted volume saved with config
- [ ] Queue/lyrics panel toggle (rightbar), External devices button (§3.10), Share current (SHARE), Mini player toggle (desktop), Fullscreen button
- [ ] `cfg.fullscreen_use_player_bar`: bar stays on top of fullscreen view (cover/title hidden in bar)
- [ ] Vaxry variant: same actions, different layout

### 3.6 Right bar: queue + lyrics (`layout/rightbar.rs`)
- [ ] Tabs "up_next" / "lyrics", close button; resizable width 280–600 px (drag left edge); mutually exclusive with devices panel
- [ ] Queue list (§3.7) in display order (shuffle permutation applied)
- [ ] Lyrics view (§3.8) for the current track

### 3.7 Queue list (`queue/list_view.rs`, `queue/drag.rs`)
Data: full mirror from `api.queue_snapshot()` (§5.1)
- [ ] Rows: cover (size by row width), title, artist, active-row indicator (volume icon), virtualized (60 px rightbar / 76 px fullscreen)
- [ ] Click row → `queue_edit(Jump{index})` (play-order index)
- [ ] Reorder by mouse drag (6 px threshold, drop above/below split at 23 px) → `queue_edit(Move{from,to})`; ▲▼ ReorderButtons per row → Move
- [ ] Drop tracks dragged from any TrackRow (single or multi-selection, custom drag preview with count) into the queue at a position or at the end → `queue_edit(Insert{index, keys})`; auto-scroll near edges; drag only enabled while rightbar open
- [ ] Summary line "current/total • total duration" (hidden for radio), click = jump-scroll to current
- [ ] Auto-follow current item; user scroll disarms; "jump_to_current_song" button re-arms
- [ ] Empty "no_more_songs"
- [ ] No remove-from-queue action exists (`QueueEdit::Remove` unused)

### 3.8 Lyrics view (`playback/lyrics.rs`, `hooks/lyrics.rs`)
Data: `api.lyrics(key)` per track change (none for radio, stale answers dropped); failure/none → Plain(t("lyrics_not_found"))
- [ ] States: "loading_lyrics", plain text, synced
- [ ] Synced: active line highlight + scale, auto-centering smooth scroll (42 % comfort offset), click line → seek to line start
- [ ] Word/syllable karaoke fill per chunk (gradient sweep + glow), chunk end = next chunk start
- [ ] Background vocal lines (dimmer, indented), opposite-turn (duet) lines right-aligned/italic, parent_line_index linkage; secondary lines stay lit while overlapping
- [ ] Instrumental interludes (gaps ≥5 s incl. intro) rendered as a note glyph that fills over the gap
- [ ] Depth blur of non-active lines (`cfg.lyrics_depth_blur`, strength `cfg.lyrics_depth_blur_strength`)
- [ ] Offset: `cfg.lyrics_offset_auto` → `PlayerState.output_latency_ms`, else `cfg.lyrics_offset_ms`
- [ ] Auto-sync disarmed by wheel/touch/scroll keys/scrollbar; "sync" button re-arms; new track re-arms
- [ ] Two layouts (rightbar, fullscreen) with separate sizing

### 3.9 Track row (`track/row.rs`) — shared by every list
- [ ] Columns: # (or track number in albums; play icon on hover), cover (`cfg.show_row_images`), title (+artist inline), artist (click → artist), album (click → album; hidden in album views), duration, dots menu; Normal and Vaxry column templates; disc headers in multi-disc albums
- [ ] Click → on_play; long-press (touch or mouse hold) → selection mode; checkbox in selection mode
- [ ] Right-click opens the same menu at the cursor
- [ ] Mouse-drag a row (or the selection) toward the queue (§3.7)
- [ ] Menu (entries gated by what the page wires): Play next (PLAY_NEXT), Add to queue (APPEND), Add to playlist (ADD_TO_PL), Remove from playlist, Download Offline / Remove Download / Downloading… (DL/UNDL), Delete (DELETE), Share (SHARE), Start radio (RADIO_T, `caps.track_radio`), View/Edit metadata
- [ ] Now-playing highlight, selection highlight, downloaded/downloading indicators

### 3.10 External devices (`playback/external_devices.rs`)
Shown only when active source has `caps.external_devices` and `authenticated`
- [ ] Bar button (highlighted when `PlayerState.external.device` is set); opens docked 320 px panel, closes rightbar
- [ ] Panel: "external_play_on", "This app" row + `api.external_devices(source_id)` (refetched on open), device icon (daemon `Icon`), kind subtitle, active marker; click → `api.select_external_device(source_id, Some(id)|None)`, then refetch; toast on error

### 3.11 Fullscreen player (`layout/fullscreen/*`)
- [ ] Opened from bar button; closed by chevron or Escape
- [ ] Background: `cfg.cover_art_background` (blurred cover, `cover_art_blur`, `cover_art_darkening`), custom image `cfg.custom_background_path`, or "album-art" theme palette gradient (palette from `api.artwork` bytes)
- [ ] Left: large Full cover, title, artist (→ artist, closes fullscreen), album (→ album), heart, don't-recommend, actions menu (Play next, Add to queue, Add to playlist if `caps.playlists != None`, Share, Start radio if `caps.track_radio`, View metadata read-only), bitrate "N kbps"
- [ ] Seek / transport / volume (fullscreen variants) unless `cfg.fullscreen_use_player_bar`
- [ ] Right tabs Up next / Lyrics; collapse/expand side panel → `cfg.fullscreen_tabs_collapsed`
- [ ] Custom titlebar also rendered inside fullscreen (Linux/Windows)

### 3.12 Compact mini-player (desktop)
- [ ] Toggled by bar button / Ctrl+M; window shrinks to 380×148 (170 on macOS), always-on-top, restores previous size on exit
- [ ] Whole surface drags window; blurred cover bg; title, artist, "mm:ss / mm:ss" or "LIVE"; restore button; prev/play-pause/next; thin seek bar with buffer indicator
- [ ] UI-style skins (Normal/Vaxry)

### 3.13 Quick Search (Ctrl/Cmd+K overlay)
- [ ] Modal input (autofocus), 150 ms debounce → `api.search`; shows top results (cover, title, artist)
- [ ] ↑/↓ select, Enter plays selected (or first once results arrive if Enter pressed early), Esc/Ctrl+K/backdrop close, click plays
- [ ] Play: fetch whole library `api.tracks(TrackFilter{sort: Fields(cfg.library_sort)}, all)` and PLAY_LIST at the hit; if not in library, PLAY_LIST(search results, i)
- [ ] "no_results_found{query}"

### 3.14 Toasts & notices
- [ ] Single transient toast, bottom-centered ~112 px up, 1.8 s, info vs error styling (`hooks::toast`)
- [ ] Sources: every mutation failure, radio starting/failed, share copied, job start failure, pin failure
- [ ] `ev:Notice` is not consumed by the Dioxus UI (port may surface it)

### 3.15 Job progress surfaces
- [ ] `use_job_progress(kind)`: seeds from `api.jobs()` (first Running of kind), then ev:JobProgress{kind} → running/phase/current/total/message, ev:JobFinished{kind} → reset
- [ ] Scan banner (shell), Library/Albums/Home sync spinners (LibrarySync), Favorites (FavoritesSync), Playlists (PlaylistSync), Downloader (UrlDownload), download overlay (Download)
- [ ] Download overlay (bottom-right): "Downloading done/total", Stop → `api.cancel_job(id)` for running Download jobs, collapsible, progress bar, "{n} failed", per-item rows from `api.download_statuses()` + titles via `api.tracks_by_keys` (Queued/Downloading/Failed icons)

### 3.16 Theme application
- [ ] `#app-root` class `theme-<cfg.theme>`; "album-art" → `theme-default` + palette gradient background; "live" → `theme-default theme-live`
- [ ] Custom themes (`cfg.custom_themes`) injected as `.theme-<id>` CSS
- [ ] Live theme (`cfg.theme == "live"`, desktop): matugen/pywal JSON at `cfg.live_theme_path` (or default) polled every 400 ms (2 s idle), recolors in place
- [ ] Custom UI font `cfg.custom_font_path` (ttf/otf/woff/woff2, ≤32 MB) applied app-wide
- [ ] `cfg.reduce_animations` → reduced-animation stylesheet
- [ ] `cfg.ui_style` Normal vs Vaxry switches sidebar, bar, showcase, headers, compact skin, page paddings

### 3.17 Tray / close behavior (desktop)
- [ ] `cfg.minimize_to_tray`: tray icon with "Show / Hide Kopuz" and "Quit Kopuz"; window close hides instead of quitting; Linux checks appindicator libs and warns/popups if missing
- [ ] On quit: final `set_config` flush (volume folded in) on a fresh thread

---

## 4. Settings

Category → rows. **[H]** = hardcoded `Settings` field (applied to the local copy at once, persisted as a typed `SettingsEdit` by the debounced `api.edit_config`); **[S]** = daemon `FieldSpec` schema form via `SchemaForm`; **[A]** = explicit API calls. Every [H] row takes `config_key` and renders locked when that key is in `ConfigView.locks`.

### 4.1 General
- [ ] [H] `auto_check_updates` toggle
- [ ] [H] `minimize_to_tray` toggle (desktop)
- [ ] [H] `back_behavior` RewindThenPrev / AlwaysPrev
- [ ] Build info card

### 4.2 Appearance (Customization)
- [ ] [H] `language` selector (`i18n::available_languages()`, applies `i18n::set_locale` immediately)
- [ ] [H] `theme` selector: album-art, default, amoled, live (desktop), gruvbox, gruvbox-classic, gruvbox-dark-soft, dracula, nord, catppuccin, ef-night, ayu-dark, ayu-mirage, vague, onedarkpro, osmium, kanagawa-dragon, everforest, rosepine, kettek16, default-light, catppuccin-latte, rosepine-dawn, everforest-light, ayu-light, one-light, gruvbox-light, + custom themes
- [ ] [H] `live_theme_path` (only when theme = live): shows resolved path, Remove, "Choose palette" (JSON file picker)
- [ ] [H] `cover_art_background` toggle
- [ ] [H] `custom_background_path` image picker + Remove (desktop)
- [ ] [H] `custom_font_path` font picker + Remove (desktop)
- [ ] [H] `cover_art_darkening` 0–95 % and `cover_art_blur` 0–100 px (only when a cover/custom background is on)
- [ ] [H] `lyrics_depth_blur` toggle; `lyrics_depth_blur_strength` 10–200 % (when on)
- [ ] [H] `reduce_animations` toggle
- [ ] [H] `fullscreen_use_player_bar` toggle (desktop)
- [ ] [H] `show_source_toggle` toggle
- [ ] [H] `show_row_images` toggle
- [ ] [H] `titlebar_mode` Custom/System/Off (Linux/Windows)
- [ ] [H] `ui_style` Normal/Vaxry
- [ ] [H] `player_bar_position` Bottom/Top
- [ ] [H] `settings_layout` Cd/TopBar
- [ ] Theme editor section (§2.15) → `custom_themes`

### 4.3 Library
- [ ] [H] Local libraries (`music_directory` + `local_sources`, locked on either key): default "Local" with folders (add via folder picker / remove), "Switch to local library" → `api.switch_source("local")`; named libraries each with folders, switch, delete (`cfg.remove_local_source`); "Add local library" popup (name + multi-folder picker, validation "local_library_name_required"/"local_library_folder_required") → `cfg.add_local_source` + `cfg.set_active_local_source`. Rescan is triggered by the shell watching these dirs (§5.9)
- [ ] [H]+[A] Radio registries dropdown (`radio_registries`): "N of M active", enable toggle (enabling validates via `api.validate_radio_registry(url)`, error "radio_registry_enable_failed"), delete (non-default only), Add popup (URL; empty/duplicate checks; `validate_radio_registry` then push `RegistryEntry{url, enabled:true, is_default:false}`)
- [ ] [A] Media servers (`id=settings-media-servers`): list of rows whose `service` is set: name, "service{name}", active badge with live status (Connected/Connecting/Disconnected + Login button when offline), Switch (`api.switch_source`, then per `SourceInfo.sign_in`: Browser → `api.authenticate_source(id)`, Password → LoginPopup), Delete (`api.delete_server`)
- [ ] [S] Active server's own options: `SourceInfo.settings` → `api.set_source_settings(id, [value])`
- [ ] [A] Remote music folders (active server with `caps.browse_folders` and authenticated): list + remove, browser (`api.browse_source(id, path)`, parent folder, "use this folder", loading/error/empty) → `api.set_source_directories(id, dirs)`
- [ ] Host access warnings when `api.can_open_browser()` is false ("browser_playback_needs_host", "browser_sign_in_needs_host", copyable)
- [ ] [A]+[S] Add server popup: name, service select from `api.services()` (with "(experimental)"), SchemaForm over `ServiceInfo.fields` (values + write-only secrets), live `api.check_server_draft` on every edit (problems shown per field / form-level); Save → `check_server_draft` → `api.upsert_server(draft)` → switch + sign-in as above
- [ ] [A] LoginPopup ("login_to_service{service}", username/password, "logging_in") → `api.login_source{server_id, username, password}`; "username_and_password_required", "login_failed{error}"
- [ ] Sign-in errors "signin_failed{error}" also surface in the playback-error banner

### 4.4 Connectivity (integrations)
- [ ] [S] For each `api.integrations()` (refetched on inv:Servers or after change): SchemaForm over `IntegrationInfo.fields` → `api.set_integration_settings(id, [value])`
- [ ] [A] `ConnectKind::WebSignIn` → button "integration_connect{name}" / "integration_connected{name}" → `api.authenticate_integration(id)`

### 4.5 Offline downloads
- [ ] [H] `offline_quality` select (`OfflineQuality::ALL`, value_str/label)

### 4.6 Metadata
- [ ] [S] Artwork provider settings: `api.artwork_settings()` → `api.set_artwork_settings([value])`
- [ ] [H] `prefer_local_lyrics`, `enable_musixmatch_lyrics` toggles
- [ ] [H] `lyrics_offset_auto` toggle (label shows measured output latency); `lyrics_offset_ms` slider ±`LYRICS_OFFSET_LIMIT_MS` step 50 (disabled when auto)

### 4.7 Player
- [ ] [H] `crossfade_seconds` 0–12 ("crossfade_off")
- [ ] [H] `volume_scroll_step` 1–50 %
- [ ] [H] `channel_mode` Stereo/Mono/LeftOnly/RightOnly/SwapLeftRight
- [ ] [H] `device_change_behavior` Resume/Pause
- [ ] [H] `sample_rate_mode` System/Source
- [ ] [H]+[A] Equalizer panel (`equalizer`): enable/disable, preset (Flat, BassBoost, TrebleBoost, VocalBoost, Loudness, Custom; animated band interpolation; preset preamp default), preamp slider, 10 band sliders on a curve graph; every change → `api.preview_equalizer(eq)` live, commit writes `cfg.equalizer`

### 4.8 Tools / Logs (desktop)
- [ ] [H] `tracing_enabled` toggle + warning
- [ ] Open logs folder (`utils::logs::open_log_dir`), Export logs (save dialog → `utils::logs::export_logs`) — local filesystem, not API
- [ ] Debug build: "Trigger crash" button; DebugPanel (app-supplied, direct DB handle, debug only)

### 4.9 SchemaForm renderer (`forms/schema_form.rs`) — must match exactly
- [ ] `FieldKind`: Text, Secret (password input, never prefilled), Url, Toggle ("true"/"false"), Directory (text + folder picker), Choice{options, custom} (select + "custom_manual" free-text when value not in list), Radio{options}, Note (help text only)
- [ ] `show_when` hides rows unless another field's current value matches; `section` heading drawn once per group; `help` under control; `placeholder`; per-field and form-level `Problem`s; `config_key` locks the row
- [ ] `Text::Key` → i18n lookup, `Text::Literal` verbatim; `Icon::Class` = FontAwesome class, `Icon::Svg` = 24×24 path

---

## 5. Cross-cutting mechanics

### 5.1 Player/queue mirror (`hooks/session_projector.rs`, `use_player_controller.rs`)
- [ ] On start and on ev:QueueChanged or ev:Resync: `api.queue_snapshot()` then `api.player_state()` (full resync; no windowing, `queue_window` unused)
- [ ] ev:PlayerState → apply: `is_playing` = Loading→true, Committed→phase==Playing, Stopped→false; `loading` = Intent::Loading; volume; output_latency_ms; playback_error = `error.message`; external_device = `external.device`; shuffle; loop_mode; current index
- [ ] Crossfade: while `fading` is Some, show `fading.track` and drive seek from `fading.position_ms`
- [ ] Now-playing fields from `track` (title, artist, album, khz, bitrate, duration secs or u64::MAX for none, artwork ref); `current_track_snapshot` = queue row with matching `uid` (not key); Intent::Stopped with no track clears metadata
- [ ] Clock: `DaemonClock` maps daemon `now_ms` to local Instant; position anchor `{ms, at_ms, playing}` interpolated locally; 1 s ticker updates progress while playing; ev:PlayerPosition re-anchors; ev:PlayerBuffered updates ranges
- [ ] Queue mirror: `items` (play order), `shuffle_order` (unshuffled index per position), `position`, `shuffle`; displayed order applies shuffle permutation
- [ ] Optimistic writes: `is_playing` on Play/Pause/Toggle, `shuffle`, `loop_mode`, `volume`, seek progress + anchor; everything else waits for events
- [ ] Row click semantics: `JumpPhysical{index}` for track-list clicks under shuffle; `Jump{index}` for queue view
- [ ] `CommandAck.rev` is never awaited anywhere in the Dioxus frontend; correctness relies on the resync-on-QueueChanged path (port may choose to wait for rev)
- [ ] ev:ConfigChanged, ev:SourceStatus, ev:Notice are not consumed by the Dioxus frontend

### 5.2 Library reactivity (`hooks/db_reactivity.rs`)
- [ ] Per-table generation counters for Tracks, Albums, Playlists, Favorites, Folders, Servers, Recents; `Table::Unknown` ignored
- [ ] ev:LibraryInvalidated{table} → `bump_coalesced` (dirty flag flushed by a 150 ms ticker = ≤~6 refreshes/s during scans/syncs)
- [ ] Each query keyed on (inputs, generation(s)); keeps stale value while re-running; windowed track queries pair rows with the offset they were fetched at
- [ ] Table → reads: Tracks (tracks window/count, album/artist/genre tracks, tracks_by_keys, artists, artist samples, top genre, downloads list, search), Albums (albums, album, recently added also Tracks), Playlists+Folders (playlists), Favorites (favorites, is_favorite), Recents (recent_tracks), Servers (sources, integrations)
- [ ] Active source (`cfg.active_source`) is also a re-run key for queries so a switch refetches everything
- [ ] Capabilities: active `SourceInfo.capabilities` held in a synchronous signal; every affordance gate reads it

### 5.3 Config lifecycle
- [ ] Startup: `api.config()` → `cfg` + locale; only if that succeeded is saving armed (never persist a defaulted config)
- [ ] Any `cfg` change or persisted-volume change marks dirty; saver loop: 600 ms settle → `api.set_config(cfg with current volume)` → 2.5 s cooldown
- [ ] Locked keys read once from `ConfigView.locks`
- [ ] After `switch_source`, pull `api.config()` back into `cfg`
- [ ] Final flush on window close/quit

### 5.4 Artwork
- [ ] Every row carries `Option<ArtworkRef{target, version}>`; None → placeholder, never request
- [ ] Dioxus serves `artwork://api?<kind>=<id>[&hq=1]&v=<version>` (Windows: `http://artwork.dioxus.localhost/api?…`) via a custom protocol backed by `api.artwork(ArtworkRequest{target, hq})`; port should fetch bytes with the same request and cache by (target, version, hq) forever
- [ ] Sizes: Thumb for rows/tiles, Full (`hq`) for hero, fullscreen, detail headers, backgrounds (`size_for(width>512)`)
- [ ] Palette extraction for album-art theme from `api.artwork(current, hq:false)` bytes
- [ ] User-picked local paths (custom background) render directly from disk
- [ ] Uploads: file picker → bytes → `api.upload_artwork{target, content_type by extension (png/webp/else jpeg), bytes}`; removal `api.remove_artwork(target)`; tag-embedded cover via `TrackMetadataPatch.cover` (Keep/Remove/Set)

### 5.5 Metadata modal (`track/metadata_modal.rs`)
- [ ] Read view: cover (Full), title/artist/album/track#/disc#, duration, sample rate (kHz), bitrate, MusicBrainz release/recording/track ids, "path" (shows `uid`)
- [ ] Edit mode (only where `caps.edit_tags`): text fields, blank number = clear flag, cover add/change (image picker, data-URL preview)/remove, warning text → `api.update_track_metadata(patch)`

### 5.6 Selection & drag
- [ ] Selection keyed by track `key` (pages) / `uid` (some components; `keys_for_uids` bridges)
- [ ] SelectionBar: "{n} selected", Add to queue, Add to playlist, Delete (gated), Cancel
- [ ] Drag payload is a process-global track list (not DataTransfer); custom floating preview (cover/title or count)

### 5.7 Scroll persistence
- [ ] Main scroll area position per Route; Album/Artist detail per `album:<id>` / `artist:<name>`; restored on navigation (skipped when a Settings anchor is pending)
- [ ] Inner scrollers: Library/Favorites/Activity per Route; album grid "albums", artist grid "artists" (process-lifetime map, restored once after first render)
- [ ] Settings anchor scrolls to element id once, then clears

### 5.8 Virtualization
- [ ] Fixed row heights (60 px lists, 60/76 queue), 10-row buffer each side; Library/Activity fetch only the visible `Page{offset,limit}`; Favorites/Search/Genre virtualize client-side lists

### 5.9 Library scan trigger (shell)
- [ ] When local library dirs change (`music_directory` + `local_sources`) or Library refresh pressed → start a scan (Dioxus calls the in-process `LibraryService.spawn_scan_with_config` directly, see §6); `JOB(Scan)` is the API route

### 5.10 Connectivity probe (shell)
- [ ] Only while a Server source is active: GET https://1.1.1.1 every 30 s (10 s while offline), offline after 2 misses; drives `is_offline` context (offline filtering on Home/Albums/Artists/Playlists) and banner

### 5.11 i18n
- [ ] Fluent, 28 locales (`apps/common/i18n/locales/*.ftl`, `en.ftl` baseline ~520 keys), ~244 distinct literal keys used by these crates plus dynamic ones (`sidebar item keys`, `Text::Key` from daemon specs, radio station/stream names, sort field `label_key`s)
- [ ] API: `t(key)`, `t_with(key, &[(arg, value)])`, `set_locale`, `available_languages`, `is_rtl`
- [ ] Hardcoded English strings still in UI (port can keep or key them): "Downloading…", "Remove Download", "Download Offline", "Copied link", "Couldn't find a page for this track", "Couldn't update some favorites", "{n} selected", "Stop", "{n} failed", "Downloading d / t", "Server • N tracks", "N playlists", "Loading tracks — …", "N playlists synced", "Refresh", "Remove downloads", "Download Album", "Go to artist", "Share", "Scanning library…", offline/online banner texts, tray menu items, "Trigger crash (debug)"

### 5.12 Theme tokens
- [ ] Every theme class redefines 13 CSS vars; custom/live themes map editor keys to them (`utils::themes::VAR_MAP`):
  - `bg`→`--color-black`, `text`→`--color-white`, `text-muted`→`--color-slate-400`, `surface`→`--color-slate-500`, `progress`→`--color-green-500`, `accent-soft`→`--color-indigo-400`, `accent`→`--color-indigo-500`, `accent-alt`→`--color-indigo-600`, `accent-deep`→`--color-indigo-900`, `highlight`→`--color-purple-600`, `highlight-dark`→`--color-purple-700`, `danger`→`--color-red-400`, `raised`→`--color-neutral-900`
- [ ] Also used: `--color-primary` (scan/buffer bars, fallback #6366f1), `--vaxry-sidebar-fg`, switcher `--accent`/`--ss-surface`/`--ss-fg`, `--color-amber-*` (folder tiles)
- [ ] Built-in theme classes (`kopuz/assets/themes.css`, 25): default, default-light, amoled, gruvbox, gruvbox-classic, gruvbox-dark-soft, gruvbox-light, dracula, nord, catppuccin, catppuccin-latte, ef-night, ayu-dark, ayu-mirage, ayu-light, vague, onedarkpro, one-light, osmium, kanagawa-dragon, everforest, everforest-light, rosepine, rosepine-dawn, kettek16; plus pseudo-themes `album-art` (default + cover palette gradient) and `live` (default + polled palette)
- [ ] Editor defaults: bg #0f0f17, raised #1a1a2a, surface #282838, text #e2e2f0, text-muted #7878a0, accent #5f8aff, accent-soft #8faeff, accent-alt #3a5fd9, accent-deep #0a0a1a, highlight #c77dff, highlight-dark #9d4edd, progress #5f8aff, danger #ff6b6b

---

## 6. Platform-specific / out-of-scope candidates

### Android-only (`cfg(target_os = "android")`)
- Media-notification buttons via JNI (`hooks/use_player_task.rs`, links `player::systemint`), shuffle/repeat mirrored to the notification
- Drawer sidebar (overlay, hamburger header with page title/back arrow, swipe-right-to-open / swipe-left-to-close), safe-area insets
- Mini bottom bar (swipe up = fullscreen, left/right = next/prev) and tabbed pull-to-dismiss fullscreen sheet (`fullscreen/android.rs`)
- Folder add via storage permission + Music-dir auto-detect (`settings/sources.rs`)
- WebView script engine for the daemon (`app_lifecycle::use_webview_script_engine`), edit-ack JS patch, Android TLS init, sync `artwork://` handler
- Excluded on Android: Downloader route, ThemeEditor, Tools/Logs category, live theme, custom background/font, tray, fullscreen_use_player_bar, compact mode, titlebar

### Desktop-OS specific
- Linux: resize handles, WebKit page-cache off, `WEBKIT_FORCE_VBLANK_TIMER`, appindicator tray probe
- Windows: titlebar WM_NCHITTEST subclass, toolbar icon assets
- macOS: title-hidden window, app menu, silent AudioContext keep-alive, drag regions, compact height 170
- Native dialogs via `rfd` (file/folder/save pickers)

### Frontend code that reaches past (or leaks around) the `KopuzApi` seam
- `kopuz/src/main.rs` App shell uses the in-process core directly: `core.session.subscribe()` (event bridge for invalidations + scan progress instead of `api.events()`), `LibraryService::spawn_scan_with_config` (scan trigger), `favorites.nudge_activate()`, `scrobbler.drain_queue(&cfg)`, debug DB panel (`core.db`); plus `reqwest` update check (GitHub) and 1.1.1.1 connectivity probe
- Offline state read from `cfg.offline_tracks` + `std::path::Path::exists()` on daemon file paths in Home, Library, Albums (grid, detail, remote detail), Artists, Playlists, Favorites, Search results, Genre detail, Showcase (both styles) — should be `api.downloads()` (`hooks::downloads::is_stored`)
- Activity play counts from `cfg.listen_counts` + `Source::listen_count_key` instead of `api.stats()`
- Local libraries added/removed/edited by mutating `AppConfig` (`add_local_source`, `remove_local_source`, `music_directory`) instead of `api.upsert_local_source` / `delete_local_source`
- Service-specific branch: DiscoverPlaylistDetail treats ids starting with `"MPRE"` (YouTube album browse ids) as albums
- Logs open/export via `utils::logs`, live theme file read via `utils::live_theme` (local FS; acceptable client-side)
- Android `hooks` crate depends on `player`

### API surface unused by the Dioxus frontend
`queue_window`, `genres`, `stats`, `upsert_local_source`, `delete_local_source`, `provision_credentials`, `clear_credentials`, `clear_integration`, `QueueEdit::Remove`; events `ConfigChanged`, `SourceStatus`, `Notice` unhandled; `CommandAck.rev` ignored.

### Suspected bugs worth not porting blindly
- Lyrics and the metadata "path" row pass `TrackInfo.uid` where the API documents `key` (`api.lyrics(key)`)
- Remote album detail (YtAlbumDetail) adds `track.uid` to playlists instead of `track.key`
- Search/genre track-row delete always `from_disk: true` without `caps.delete_from_disk` gating
- Vaxry showcase "Shuffle" toggles shuffle AND plays shuffled; Normal showcase shuffle only toggles
