--- The example plugin's pretend backend.
---
--- `init.lua` holds the contract with the host; this module holds the part a
--- real plugin replaces with HTTP calls. The table shapes are the same either
--- way, so the request lives here and the mapping lives there.
---
--- Ids below are bare refs. The host namespaces them as `example/<ref>` before
--- they reach the database and strips the prefix before handing them back, so
--- nothing in this file mentions the plugin id. A ref may not contain `:`: the
--- app's playback-ref parser splits on it.
---
--- The recordings are public domain, served by Wikimedia Commons. The titles and
--- years wrapped around them are invented, which is the only honest way to fake
--- a catalog.

local M = {}

local function commons(path)
  return "https://upload.wikimedia.org/wikipedia/commons/" .. path
end

--- A cover *ref*, which is not the same string as an image URL: the
--- `directurl:` prefix tells the host to fetch the URL as it stands instead of
--- asking a configured server for the image.
local function cover_ref(url)
  return "directurl:" .. url
end

--- Artist photos, by contrast, are plain URLs.
local PHOTOS = {
  chopin = commons("thumb/e/e8/Frederic_Chopin_photo.jpeg/500px-Frederic_Chopin_photo.jpeg"),
  bach = commons("thumb/6/6a/Johann_Sebastian_Bach.jpg/500px-Johann_Sebastian_Bach.jpg"),
  mozart = commons("thumb/1/1e/Wolfgang-amadeus-mozart_1.jpg/500px-Wolfgang-amadeus-mozart_1.jpg"),
}

M.artist_images = {
  { name = "Frédéric Chopin", image = PHOTOS.chopin },
  { name = "Johann Sebastian Bach", image = PHOTOS.bach },
  { name = "Wolfgang Amadeus Mozart", image = PHOTOS.mozart },
}

M.albums = {
  {
    album_id = "nocturnes",
    title = "Nocturnes",
    artist = "Frédéric Chopin",
    year = 1837,
    genre = "Classical",
    cover = cover_ref(PHOTOS.chopin),
  },
  {
    album_id = "cello-suites",
    title = "Cello Suites",
    artist = "Johann Sebastian Bach",
    year = 1720,
    genre = "Classical",
    cover = cover_ref(PHOTOS.bach),
  },
  {
    album_id = "piano-sonatas",
    title = "Piano Sonatas",
    artist = "Wolfgang Amadeus Mozart",
    year = 1788,
    genre = "Classical",
    cover = cover_ref(PHOTOS.mozart),
  },
}

--- Only `item_id` and `title` are required. Everything else improves the row the
--- user sees, and `album_id` is what puts the track on an album page.
M.tracks = {
  {
    item_id = "chopin-op27-1",
    title = "Nocturne No. 7 in C-sharp minor, Op. 27 No. 1",
    artist = "Frédéric Chopin",
    artists = { "Frédéric Chopin" },
    album = "Nocturnes",
    album_id = "nocturnes",
    cover = cover_ref(PHOTOS.chopin),
    duration_secs = 49,
    khz = 44100,
    bitrate = 112,
    track_number = 1,
    disc_number = 1,
  },
  {
    item_id = "bach-bwv1007-sarabande",
    title = "Cello Suite No. 1 in G major, BWV 1007: IV. Sarabande",
    artist = "Johann Sebastian Bach",
    artists = { "Johann Sebastian Bach" },
    album = "Cello Suites",
    album_id = "cello-suites",
    cover = cover_ref(PHOTOS.bach),
    duration_secs = 144,
    khz = 44100,
    bitrate = 338,
    track_number = 4,
    disc_number = 1,
  },
  {
    item_id = "mozart-k457-i",
    title = "Piano Sonata No. 14 in C minor, K. 457: I. Molto allegro",
    artist = "Wolfgang Amadeus Mozart",
    artists = { "Wolfgang Amadeus Mozart" },
    album = "Piano Sonatas",
    album_id = "piano-sonatas",
    cover = cover_ref(PHOTOS.mozart),
    duration_secs = 254,
    khz = 44100,
    bitrate = 136,
    track_number = 1,
    disc_number = 1,
  },
}

--- item_id to track, built once at load.
M.by_id = {}
for _, track in ipairs(M.tracks) do
  M.by_id[track.item_id] = track
end

--- What `resolve_stream` answers with. The lengths are what the server actually
--- reports: the host reads the whole file with a buffered GET and drives
--- scrubbing off `Content-Length`, so a wrong number costs seeking.
M.streams = {
  ["chopin-op27-1"] = {
    url = commons("e/e0/Chopin_Nocturne_Op.27_No.1.oga"),
    content_length = 692632,
    duration_secs = 49,
  },
  ["bach-bwv1007-sarabande"] = {
    url = commons("4/4b/Bach_-_Cello_Suite_no._1_in_G_major%2C_BWV_1007_-_IV._Sarabande.ogg"),
    content_length = 6083909,
    duration_secs = 144,
  },
  ["mozart-k457-i"] = {
    url = commons("8/86/Mozart_-_Piano_Sonata_No._14.ogg"),
    content_length = 4311313,
    duration_secs = 254,
  },
}

--- Playlist ids stay opaque: unlike track and album ids they are never
--- persisted under a namespace, so whatever shape the backend uses survives.
M.playlists = {
  {
    playlist_id = "late-night",
    name = "Late night",
    image = cover_ref(PHOTOS.chopin),
  },
}

--- A track as it appears inside a playlist. `playlist_item_id` is the backend's
--- handle for the entry rather than for the track, which is what a removal has
--- to name when the same track sits in the playlist twice.
local function entry(item_id, playlist_item_id)
  local track = {}
  for key, value in pairs(M.by_id[item_id]) do
    track[key] = value
  end
  track.playlist_item_id = playlist_item_id
  return track
end

local ENTRIES = {
  ["late-night"] = {
    entry("chopin-op27-1", "late-night-1"),
    entry("mozart-k457-i", "late-night-2"),
    entry("bach-bwv1007-sarabande", "late-night-3"),
  },
}

--- The tracks in a playlist, in order, or nil when there is no such playlist.
function M.playlist_entries(playlist_id)
  return ENTRIES[playlist_id]
end

--- Tracks whose title or artist contains `needle`, already lowercased. `find`
--- runs in plain mode so a query with `-` or `(` in it is not a Lua pattern.
function M.matching(needle, limit)
  local hits = {}
  for _, track in ipairs(M.tracks) do
    if #hits >= limit then
      return hits
    end
    if track.title:lower():find(needle, 1, true) or track.artist:lower():find(needle, 1, true) then
      hits[#hits + 1] = track
    end
  end
  return hits
end

--- Albums whose title or artist contains `needle`.
function M.albums_matching(needle)
  local hits = {}
  for _, album in ipairs(M.albums) do
    if album.title:lower():find(needle, 1, true) or album.artist:lower():find(needle, 1, true) then
      hits[#hits + 1] = album
    end
  end
  return hits
end

return M
