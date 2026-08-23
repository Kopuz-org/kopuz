# Matugen and pywal

[matugen](https://github.com/InioX/matugen) and [pywal](https://github.com/dylanaraps/pywal)
build a colour scheme out of your wallpaper and rewrite their output whenever
that wallpaper changes. Kopuz can follow along: pick **Matugen / Pywal (live)**
in *Settings -> Appearance* and it recolours itself within half a second of the
palette being regenerated. No restart, and nothing to hook up on their side.

## pywal

Nothing to configure. Kopuz reads `~/.cache/wal/colors.json` where pywal already
writes it, so `wal -i ~/wallpapers/whatever.png` is enough.

## matugen

matugen has no fixed output, so it needs a template. Save this as
`~/.config/matugen/templates/kopuz.json`:

```json
{
  "bg":             "{{colors.surface.default.hex}}",
  "text":           "{{colors.on_surface.default.hex}}",
  "text-muted":     "{{colors.on_surface_variant.default.hex}}",
  "surface":        "{{colors.outline.default.hex}}",
  "progress":       "{{colors.primary.default.hex}}",
  "accent-soft":    "{{colors.primary_fixed_dim.default.hex}}",
  "accent":         "{{colors.primary.default.hex}}",
  "accent-alt":     "{{colors.tertiary.default.hex}}",
  "accent-deep":    "{{colors.primary_container.default.hex}}",
  "highlight":      "{{colors.secondary.default.hex}}",
  "highlight-dark": "{{colors.secondary_container.default.hex}}",
  "danger":         "{{colors.error.default.hex}}",
  "raised":         "{{colors.surface_container.default.hex}}"
}
```

and register it in `~/.config/matugen/config.toml`:

```toml
[templates.kopuz]
input_path = "~/.config/matugen/templates/kopuz.json"
output_path = "~/.config/kopuz/matugen.json"
```

The `.default` tokens track matugen's active light or dark mode, so one template
covers both.

## Palette location

Kopuz looks for `matugen.json` in its config directory first, then pywal's
`colors.json`. The path it settled on is shown in the settings row, and *Choose
Palette* overrides it if you keep yours somewhere else.

| Platform | Config directory |
| --- | --- |
| Linux | `~/.config/kopuz` |
| macOS | `~/Library/Application Support/moe.kopuz.kopuz` |
| Windows | `%APPDATA%\kopuz\kopuz\config` |

## Notes

- The keys are the ones `crates/kopuz/assets/themes.json` uses. Extras are
  ignored and anything missing keeps its default colour, so a partial palette
  still works.
- The file is only polled while the theme is selected.
- None of this is specific to those two tools. Anything that can write that JSON
  drives the theme the same way.
