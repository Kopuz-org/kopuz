# Kopuz radio registry guide

This is a guide for writing your own radio registry for Kopuz. A registry is a small set of JSON files that tells Kopuz which stations exist, where to stream them, and how to fetch now-playing metadata. You can host one on GitHub Pages, an S3 bucket, or any plain HTTPS file server.

No build step, no framework. Just JSON files.

---

## File layout

A registry has one index file and one manifest file per station:

```
my-registry/
  index.json
  stations/
    my_station.json
    another_station.json
```

The index lists stations. Each station's manifest file describes the station itself. Kopuz fetches the index first, then fetches each manifest listed inside it.

---

## Registry index

`index.json` is the entry point for the whole registry.

### Fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `registry_name` | string | yes | Human-readable name for the registry. |
| `description` | string | yes | Short description of what is in it. |
| `stations` | array | yes | List of station references (see below). |

Each entry in `stations` has:

| Field | Type | Required | Notes |
|---|---|---|---|
| `id` | string | yes | Must match the `id` inside the station manifest. |
| `manifest_url` | string | yes | Relative or absolute URL to the manifest JSON. |

### Example

```json
{
  "registry_name": "My Radio Registry",
  "description": "Japanese internet radio stations I listen to.",
  "stations": [
    { "id": "listen_moe", "manifest_url": "./stations/listen_moe.json" },
    { "id": "cafe_radio", "manifest_url": "./stations/cafe_radio.json" }
  ]
}
```

---

## Station manifest

Each station has its own JSON file covering three things: identity, streams, and metadata.

### Top-level fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `schema_version` | string | yes | Must be `"1"` or `"1.0"`. |
| `id` | string | yes | Alphanumeric, underscores, and dashes only. Must match the id in the registry index. |
| `name` | string | yes | Display name shown in Kopuz. |
| `description` | string | yes | Short description of the station. |
| `icon` | string | no | Font Awesome icon class. Defaults to `"fa-solid fa-radio"`. |
| `tags` | array | no | Free-form strings, e.g. `["jpop", "anime"]`. |
| `streams` | array | yes | One or more audio stream definitions. |
| `metadata` | object | no | Now-playing metadata source. Omit if you have none. |

### Stream fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `id` | string | yes | Unique within this manifest. |
| `name` | string | yes | Shown in the stream picker. |
| `url` | string | yes | Must start with `https://` or `wss://`. |
| `codec` | string | no | Audio codec hint, e.g. `"mp3"`, `"aac"`, `"opus"`. |
| `bitrate` | number | no | Bitrate in kbps. |
| `icon` | string | no | Per-stream icon override. |

---

## Metadata sources

The `metadata` field is a discriminated union on the `type` key. Three types are supported:

| Type | When to use |
|---|---|
| `static` | No API exists, or the stream plays a fixed programme. |
| `rest` | A JSON HTTP endpoint you can poll. |
| `websocket` | The station pushes updates over a WebSocket connection. |

---

### `type: "static"`

Kopuz reads the metadata once when playback starts and never updates it. Good for 24/7 looping streams, or stations with no API.

#### Fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `type` | string | yes | `"static"` |
| `title` | string | yes | Fallback track title for all streams with no override. |
| `artist` | string | yes | Fallback artist name. |
| `cover_url` | string | no | Fallback cover art URL. Must start with `https://`. |
| `stream_overrides` | object | no | Per-stream metadata, keyed by stream id. |

Each entry in `stream_overrides` has:

| Field | Type | Required | Notes |
|---|---|---|---|
| `title` | string | yes | Track title for this stream. |
| `artist` | string | yes | Artist name for this stream. |
| `cover_url` | string | no | Cover art URL for this stream. Must start with `https://`. |

If a stream has no entry in `stream_overrides`, the top-level `title`, `artist`, and `cover_url` are used.

#### Single stream example

```json
{
  "schema_version": "1.0",
  "id": "lofi_chill",
  "name": "Lo-Fi Chill Radio",
  "description": "24/7 lo-fi hip-hop beats.",
  "icon": "fa-solid fa-headphones",
  "tags": ["lofi", "chill", "hiphop"],
  "streams": [
    {
      "id": "main",
      "name": "128k MP3",
      "url": "https://stream.example.com/lofi-128",
      "codec": "mp3",
      "bitrate": 128
    }
  ],
  "metadata": {
    "type": "static",
    "title": "Lo-Fi Beats 24/7",
    "artist": "Various Artists",
    "cover_url": "https://cdn.example.com/lofi-cover.jpg"
  }
}
```

#### Multi-stream example with per-stream overrides

```json
{
  "schema_version": "1.0",
  "id": "cafe_radio",
  "name": "Café Radio",
  "description": "Ambient streams, no live now-playing API.",
  "streams": [
    { "id": "jazz",      "name": "Jazz",      "url": "https://stream.example.com/jazz"      },
    { "id": "classical", "name": "Classical", "url": "https://stream.example.com/classical" },
    { "id": "ambient",   "name": "Ambient",   "url": "https://stream.example.com/ambient"   }
  ],
  "metadata": {
    "type": "static",
    "title": "Café Radio",
    "artist": "Café Ensemble",
    "cover_url": "https://cdn.example.com/cafe-cover.jpg",
    "stream_overrides": {
      "jazz": {
        "title": "Jazz & Bossa Nova Lounge",
        "artist": "Jazz Collective",
        "cover_url": "https://cdn.example.com/jazz-cover.jpg"
      },
      "classical": {
        "title": "Classical Morning",
        "artist": "Various Classical Artists"
      }
    }
  }
}
```

The `ambient` stream has no override, so it falls back to the top-level title and artist.

---

### `type: "rest"`

Kopuz polls an HTTPS endpoint on a fixed interval. It parses the JSON response using dot-notation paths you define in `mapping`.

#### Fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `type` | string | yes | `"rest"` |
| `url` | string | yes | Default metadata endpoint. Must start with `https://`. Supports `{stream_id}` placeholder. |
| `mapping` | object | yes | Dot-notation paths for extracting metadata (see field mapping below). |
| `poll_interval_secs` | number | no | How often to poll. Defaults to `5`. |
| `headers` | object | no | Extra HTTP headers, e.g. API keys. |
| `stream_url_map` | object | no | Per-stream URL overrides, keyed by stream id. Takes precedence over `url`. |
| `entry_selector` | object | no | Used when one API response covers multiple streams (see entry selector below). |
| `stream_name_map` | object | no | Maps stream id to display name, used by `entry_selector`. |

#### Example

```json
"metadata": {
  "type": "rest",
  "url": "https://api.example.com/nowplaying?channel={stream_id}",
  "poll_interval_secs": 10,
  "mapping": {
    "title": "song.title",
    "artist": "song.artists.0.name",
    "artwork_url": "song.albums.0.image"
  }
}
```

---

### `type: "websocket"`

Kopuz opens a persistent WebSocket connection and processes push messages as they arrive. This was built for listen.moe and may not work for other providers.

#### Fields

| Field | Type | Required | Notes |
|---|---|---|---|
| `type` | string | yes | `"websocket"` |
| `url` | string | yes | WebSocket endpoint. Must start with `wss://`. |
| `mapping` | object | yes | Dot-notation paths for extracting metadata. |
| `stream_url_map` | object | no | Per-stream WebSocket URL overrides. |
| `message_filter` | object | no | Filters messages by opcode or type field, so only relevant messages update the display. |
| `heartbeat` | object | no | Configures keep-alive messages sent to the server on a timer. |

---

## Field mapping

The `mapping` object tells Kopuz where to find the track title, artist, and artwork inside a response. Paths use dot-notation. Array indices are supported.

| Field | Type | Required | Notes |
|---|---|---|---|
| `title` | string | yes | Path to the track title. |
| `artist` | string | yes | Path to the artist name. |
| `artwork_url` | string | no | Path to a direct artwork URL, or to a value used by `artwork_url_template`. |
| `artwork_url_template` | string | no | URL template where `{value}` is replaced with the value at the `artwork_url` path. |
| `artist_array_field` | string | no | Path to an array of artist objects. Each is read using the `artist` path. |
| `artist_separator` | string | no | String used to join multiple artists. Defaults to `", "`. |

Example paths: `"song.title"`, `"song.artists.0.name"`, `"results.0.img_large_url"`.

### Example:

**Mapping config:**

```json
{
  "title": "song.title",
  "artist": "song.artists.0.name",
  "artwork_url": "song.thumb_id",
  "artwork_url_template": "https://i.example.com/{value}/400x400.jpg",
  "artist_array_field": "song.artists",
  "artist_separator": " & "
}
```

**API response:**

```json
{
  "song": {
    "title": "Cielo Azul",
    "thumb_id": "a91f3c",
    "artists": [
      { "name": "ABSOLUTE CASTAWAY" },
      { "name": "中恵光城" }
    ]
  }
}
```

**Resolved values:**

| Field | Result |
|---|---|
| `title` | `Cielo Azul` |
| `artist` | `ABSOLUTE CASTAWAY & 中恵光城` |
| `artwork` | `https://i.example.com/a91f3c/400x400.jpg` |

`artist_array_field` points to the array; `artist` is then evaluated relative to each element and the results are joined by `artist_separator`. `artwork_url` extracts the raw value (`a91f3c`), which `artwork_url_template` uses to build the final URL.

---

## Entry selector

Some APIs return one JSON response for all streams at once. `entry_selector` picks the right entry for the active stream.

| Field | Type | Required | Notes |
|---|---|---|---|
| `array_path` | string | yes | Dot-path to the array inside the response, e.g. `"channels"`. |
| `match_field` | string | yes | Field in each array element to match against. |
| `match_value_from` | string | no | `"stream_name"` (default) matches against the value in `stream_name_map`. `"stream_id"` matches against the raw stream id. |

### Example

**Station config:**
```json
{
  "schema_version": "1.0",
  "id": "j1",
  "name": "J1 Tokyo",
  "description": "radio_j1_desc",
  "icon": "fa-solid fa-radio",
  "streams": [
    { "id": "J1HITS", "name": "J1 HITS", "url": "https://jenny.torontocast.com:2000/stream/J1HITS", "icon": "fa-solid fa-fire" },
    { "id": "J1GOLD", "name": "J1 GOLD", "url": "https://jenny.torontocast.com:2000/stream/J1GOLD", "icon": "fa-solid fa-compact-disc" }
  ],
  "metadata": {
    "type": "rest",
    "url": "https://json.j1fm.tokyo/whatweplay.json",
    "poll_interval_secs": 20,
    "entry_selector": {
      "array_path": "station",
      "match_field": "name",
      "match_value_from": "stream_name"
    },
    "stream_name_map": {
      "J1HITS": "J1 HITS",
      "J1GOLD": "J1 GOLD"
    },
    "mapping": {
      "title": "title",
      "artist": "artist",
      "artwork_url": "image_url"
    }
  }
}
```

**API response (`https://json.j1fm.tokyo/whatweplay.json`):**

```json
{
  "station": [
    {
      "name": "J1 HITS",
      "title": "Specialz",
      "artist": "King Gnu",
      "image_url": "https://i.example.com/specialz.jpg"
    },
    {
      "name": "J1 GOLD",
      "title": "Lemon",
      "artist": "Kenshi Yonezu",
      "image_url": "https://i.example.com/lemon.jpg"
    }
  ]
}
```

**How `entry_selector` works:**

Kopuz reads the active stream ID (e.g. `J1HITS`), maps it to a display name via `stream_name_map` (`"J1 HITS"`), then scans the `station` array for the entry where `name` equals that value. The matched entry is then passed to `mapping`.

**Resolved values (active stream: J1 HITS):**

| Field | Path | Result |
|---|---|---|
| `title` | `title` | `Specialz` |
| `artist` | `artist` | `King Gnu` |
| `artwork` | `image_url` | `https://i.example.com/specialz.jpg` |

---

## Validation

Kopuz validates every manifest before importing. A station that fails validation is skipped.

#### Identity

- `schema_version` must be `"1"` or `"1.0"`.
- `id` must be alphanumeric with underscores or dashes only, no spaces.
- `id` in the manifest must match the `id` in the registry index.
- `name` and `description` cannot be blank.

#### Streams

- At least one stream must be defined.
- Stream ids must be unique within the manifest.
- Stream ids cannot be blank.

#### URLs

- All stream URLs must start with `https://` or `wss://`.
- REST metadata URLs must start with `https://`.
- WebSocket metadata URLs must start with `wss://`.
- Static `cover_url` fields (top-level and per-stream) must start with `https://`.

`http://` is rejected everywhere.

#### Static metadata

- `title` and `artist` cannot be blank, on the top-level or in any stream override.

---

## Publishing

Any HTTPS file host works.

#### GitHub Pages

1. Create a public repository.
2. Add `index.json` and your station manifests.
3. Enable GitHub Pages in repository settings (source: main branch, root folder).
4. Your registry URL will be `https://<username>.github.io/<repo-name>/index.json`.

Relative `manifest_url` paths like `"./stations/my_station.json"` resolve correctly from that base URL.

#### Any static file server

Put the files somewhere HTTPS-accessible. Make sure the server sends `Content-Type: application/json` for `.json` files. No CORS setup needed.

#### Locally

You can use a local path to `index.json` too.

---

## Quick checklist

Before sharing your registry:

- `index.json` has `registry_name`, `description`, and at least one station entry
- Every `id` in the index matches the `id` inside the corresponding manifest
- All stream URLs start with `https://` or `wss://`
- If using `static`, `title` and `artist` are not blank
- If using `rest`, open the API URL in a browser and trace the dot-path to your title field
- The registry index URL is reachable over HTTPS

---

## Limitations

- WebSocket support was built specifically for listen.moe. Other stations may not work.

- Metadata providers that require dynamic query parameters (timestamps, tokens, signatures, or anything generated per-request) are not supported. If you run into this, use `static` metadata instead.
