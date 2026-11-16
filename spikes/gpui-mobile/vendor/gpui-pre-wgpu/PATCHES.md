# gpui-pre-wgpu 0.3.5, patched for kopuz

Vendored from crates.io and applied through the spike workspace's `[patch.crates-io]`, so only the
Android build uses it. Every change is marked `kopuz:` in the source.

- `wgpu_atlas.rs`: on Android a default-size atlas texture keeps its pixels on the CPU, and a
  frame's new tiles go up as one copy of the rectangle around them instead of one copy per glyph or
  image. Adreno's driver allocates kernel memory for every buffer-to-image copy, so the first frames
  of a fling, which bring in a screenful of new glyphs and covers, spent 20 to 50 ms of CPU in the
  draw. Larger textures, and every other platform, upload as before.
