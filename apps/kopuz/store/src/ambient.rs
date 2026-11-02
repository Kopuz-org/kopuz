//! The light a cover casts on the window: a small field rendered off the UI thread, keyed like `palette`.

use std::collections::VecDeque;
use std::sync::Arc;

use api::ArtworkRef;
use gpui_kit::{App, Asset, Global, Hsla, RenderImage, Rgba, Window};
use image::{Frame, RgbaImage};

use crate::artwork::{ArtworkKey, thumbnail_bytes};

/// How far the light reaches: pooled where the window is sparse, or carried through the middle too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Spread {
    Glow,
    Full,
}

/// What the light falls on: the theme's background, its faintest ink, and which way deep is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ground {
    pub background: [u8; 3],
    pub ink: [u8; 3],
    pub dark: bool,
}

/// What one cover casts, as far as this frame can tell.
pub enum Cast {
    /// Still being rendered, so whatever was up should stay up.
    Pending,
    /// A cover with no colour of its own lights nothing.
    Nothing,
    Light(Arc<RenderImage>),
}

/// Cells a side: enough to keep where a cover's colours sit, too few to keep its picture.
const CELLS: usize = 8;
/// How many cells wide the bell each one is spread by; under one and the cells show as tiles.
const SOFTNESS: f32 = 1.4;
/// The field's own size; the GPU stretches it, and at this size the grain stays finer than a band.
const WIDTH: u32 = 384;
const HEIGHT: u32 = 240;
/// Fields kept at once: the one showing, the one it is fading from, and one to spare.
const KEPT: usize = 3;

/// Chroma under the first says nothing about hue; from the second up a cell speaks at full weight.
const CHROMA_QUIET: f32 = 0.06;
const CHROMA_LOUD: f32 = 0.25;
/// The lightness a cell is pinned to before it is cast: deep on a dark ground, pale on a light one.
const DEEP: f32 = 0.30;
const PALE: f32 = 0.74;
const SATURATE: f32 = 1.15;
const SATURATION_CAP: f32 = 0.85;
/// A light ground takes less of the colour, or it stops being a ground.
const LIGHT_GROUND: f32 = 0.55;

/// The wash under the top chrome, and how far down the window it has faded to nothing.
const SKY: f32 = 0.34;
const SKY_REACH: f32 = 0.62;
/// The pool behind the bar's cover, bottom left, where the playing track sits; radii in window units.
const LAMP: f32 = 0.30;
const LAMP_WIDTH: f32 = 0.42;
const LAMP_HEIGHT: f32 = 0.38;
/// Where the pool ends: a bell never reaches nothing, and the calm band is to be the theme's exactly.
const LAMP_EDGE: f32 = 0.02;
/// What `Full` carries through the middle band that `Glow` leaves to the theme.
const FLOOR: f32 = 0.30;
const PEAK: f32 = 0.40;
/// `Full` is asked for to be seen: the sky and the lamp burn hotter and may reach further.
const FULL_GAIN: f32 = 1.4;
const PEAK_FULL: f32 = 0.62;

/// WCAG AA for body text, which the faintest ink has to keep over the brightest the light gets.
const READABLE: f32 = 4.5;

/// Linear light; everything is mixed here and only encoded on the way out.
type Rgb = [f32; 3];

/// One cell of the cover: the colour it casts, and how much it has to say.
#[derive(Debug, Clone, Copy)]
struct Cell {
    colour: Rgb,
    weight: f32,
}

fn linear(channel: u8) -> f32 {
    let value = f32::from(channel) / 255.;
    match value <= 0.04045 {
        true => value / 12.92,
        false => ((value + 0.055) / 1.055).powf(2.4),
    }
}

/// Back to sRGB, on the 0..255 scale but not yet rounded: the grain goes in before that.
fn encoded(value: f32) -> f32 {
    let value = value.clamp(0., 1.);
    let curve = match value <= 0.003_130_8 {
        true => value * 12.92,
        false => 1.055 * value.powf(1. / 2.4) - 0.055,
    };
    curve * 255.
}

fn linear_rgb(colour: [u8; 3]) -> Rgb {
    [linear(colour[0]), linear(colour[1]), linear(colour[2])]
}

fn luminance(colour: Rgb) -> f32 {
    0.2126 * colour[0] + 0.7152 * colour[1] + 0.0722 * colour[2]
}

fn contrast(left: Rgb, right: Rgb) -> f32 {
    let (left, right) = (luminance(left), luminance(right));
    (left.max(right) + 0.05) / (left.min(right) + 0.05)
}

fn mix(from: Rgb, to: Rgb, amount: f32) -> Rgb {
    [
        from[0] + (to[0] - from[0]) * amount,
        from[1] + (to[1] - from[1]) * amount,
        from[2] + (to[2] - from[2]) * amount,
    ]
}

fn smooth(value: f32) -> f32 {
    let value = value.clamp(0., 1.);
    value * value * (3. - 2. * value)
}

/// The most of itself the light ever adds on this ground.
fn peak(ground: Ground, spread: Spread) -> f32 {
    let top = match spread {
        Spread::Glow => PEAK,
        Spread::Full => PEAK_FULL,
    };
    match ground.dark {
        true => top,
        false => top * LIGHT_GROUND,
    }
}

/// How much of the cover's colour lands at a point of the window, before the cell has its say.
fn strength(u: f32, v: f32, spread: Spread, ground: Ground) -> f32 {
    let sky = smooth(1. - v / SKY_REACH);
    let (across, up) = (u / LAMP_WIDTH, (1. - v) / LAMP_HEIGHT);
    let bell = (-(across * across + up * up)).exp();
    let lamp = ((bell - LAMP_EDGE) / (1. - LAMP_EDGE)).max(0.);
    let (gain, floor) = match spread {
        Spread::Glow => (1., 0.),
        Spread::Full => (FULL_GAIN, FLOOR),
    };
    let scale = match ground.dark {
        true => 1.,
        false => LIGHT_GROUND,
    };
    (((SKY * sky * sky + LAMP * lamp) * gain + floor) * scale).min(peak(ground, spread))
}

/// One cell's colour as it is cast: hue kept, lightness pinned, and never enough of it to lose the ink.
fn tone(colour: [u8; 3], ground: Ground, spread: Spread) -> Cell {
    let [r, g, b] = colour.map(|channel| f32::from(channel) / 255.);
    let chroma = r.max(g).max(b) - r.min(g).min(b);
    let weight = smooth((chroma - CHROMA_QUIET) / (CHROMA_LOUD - CHROMA_QUIET));
    let seen: Hsla = Rgba { r, g, b, a: 1. }.into();
    let cast: Rgba = Hsla {
        s: (seen.s * SATURATE).min(SATURATION_CAP),
        l: match ground.dark {
            true => DEEP,
            false => PALE,
        },
        ..seen
    }
    .into();
    let cast = [cast.r, cast.g, cast.b].map(|channel| linear((channel * 255.).round() as u8));
    Cell {
        colour: cast,
        weight: weight * reach(cast, ground, spread),
    }
}

/// The share of the peak a colour may reach while the ground's faintest ink still reads over it.
fn reach(colour: Rgb, ground: Ground, spread: Spread) -> f32 {
    let (background, ink) = (linear_rgb(ground.background), linear_rgb(ground.ink));
    let most = peak(ground, spread);
    if contrast(ink, mix(background, colour, most)) >= READABLE {
        return 1.;
    }
    let (mut readable, mut unreadable) = (0., most);
    for _ in 0..12 {
        let middle = (readable + unreadable) / 2.;
        match contrast(ink, mix(background, colour, middle)) >= READABLE {
            true => readable = middle,
            false => unreadable = middle,
        }
    }
    readable / most
}

/// The cover as a few cells of cast colour, or nothing at all if none of them has any.
fn grid(bytes: &[u8], ground: Ground, spread: Spread) -> Option<Vec<Cell>> {
    let cells = CELLS as u32;
    let small = image::load_from_memory(bytes)
        .ok()?
        .thumbnail_exact(cells, cells)
        .to_rgb8();
    let grid: Vec<Cell> = small
        .pixels()
        .map(|pixel| tone(pixel.0, ground, spread))
        .collect();
    grid.iter()
        .any(|cell| cell.weight > f32::EPSILON)
        .then_some(grid)
}

/// How much each cell along one axis has to say at `at`: a bell over the cells, summing to one.
fn shares(at: f32) -> [f32; CELLS] {
    let centre = at * CELLS as f32 - 0.5;
    let mut shares = [0.; CELLS];
    for (cell, share) in shares.iter_mut().enumerate() {
        let away = (cell as f32 - centre) / SOFTNESS;
        *share = (-away * away / 2.).exp();
    }
    let total: f32 = shares.iter().sum();
    shares.map(|share| share / total)
}

/// The field at a point. Interpolating between neighbours tiles it; a bell over every cell does not.
fn sample(grid: &[Cell], across: &[f32; CELLS], down: &[f32; CELLS]) -> Cell {
    // Weighted by what each cell has to say, so a grey cell fades the light without greying it.
    let mut colour = [0.; 3];
    let mut weight = 0.;
    for (row, down) in down.iter().enumerate() {
        for (column, across) in across.iter().enumerate() {
            let cell = grid[row * CELLS + column];
            let said = cell.weight * across * down;
            colour = [
                colour[0] + cell.colour[0] * said,
                colour[1] + cell.colour[1] * said,
                colour[2] + cell.colour[2] * said,
            ];
            weight += said;
        }
    }
    if weight > f32::EPSILON {
        colour = colour.map(|channel| channel / weight);
    }
    Cell { colour, weight }
}

/// A repeatable number in 0..1 for a pixel, so the same cover always renders the same bytes.
fn hash(x: u32, y: u32, salt: u32) -> f32 {
    let mut bits =
        x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA6B) ^ salt.wrapping_mul(0xC2B2_AE35);
    bits ^= bits >> 16;
    bits = bits.wrapping_mul(0x7FEB_352D);
    bits ^= bits >> 15;
    bits = bits.wrapping_mul(0x846C_A68B);
    bits ^= bits >> 16;
    (bits >> 8) as f32 / (1_u32 << 24) as f32
}

/// Triangular noise of one level either way: what stops eight bits of dark gradient contouring.
fn grain(x: u32, y: u32) -> f32 {
    hash(x, y, 1) + hash(x, y, 2) - 1.
}

/// The whole field for one cover on one ground, in RGBA; `None` when the cover has no colour to cast.
fn render(bytes: &[u8], ground: Ground, spread: Spread) -> Option<RgbaImage> {
    let grid = grid(bytes, ground, spread)?;
    let background = linear_rgb(ground.background);
    let [r, g, b] = ground.background;
    let place = |at: u32, of: u32| (at as f32 + 0.5) / of as f32;
    let across: Vec<_> = (0..WIDTH).map(|x| shares(place(x, WIDTH))).collect();
    let down: Vec<_> = (0..HEIGHT).map(|y| shares(place(y, HEIGHT))).collect();
    Some(RgbaImage::from_fn(WIDTH, HEIGHT, |x, y| {
        let (u, v) = (place(x, WIDTH), place(y, HEIGHT));
        let cell = sample(&grid, &across[x as usize], &down[y as usize]);
        let amount = strength(u, v, spread, ground) * cell.weight;
        // Where no light lands the ground is exactly the theme's, and a flat colour cannot band.
        if amount <= f32::EPSILON {
            return image::Rgba([r, g, b, 255]);
        }
        let lit = mix(background, cell.colour, amount);
        let noise = grain(x, y);
        let [r, g, b] = lit.map(|channel| (encoded(channel) + noise).round().clamp(0., 255.) as u8);
        image::Rgba([r, g, b, 255])
    }))
}

/// GPUI keeps its images BGRA, which is the one thing about the field it has to be told.
fn frame(mut field: RgbaImage) -> RenderImage {
    for pixel in field.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    RenderImage::new(vec![Frame::new(field)])
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FieldKey {
    artwork: ArtworkKey,
    ground: Ground,
    spread: Spread,
}

/// One rendered field; GPUI's asset cache keys it and dedupes it, as it does the palette.
enum Field {}

impl Asset for Field {
    type Source = FieldKey;
    type Output = Option<Arc<RenderImage>>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let bytes = thumbnail_bytes(&source.artwork, cx);
        async move {
            let bytes = bytes.await?;
            let span = tracing::debug_span!("ambient.render");
            let _entered = span.enter();
            render(&bytes, source.ground, source.spread).map(|field| Arc::new(frame(field)))
        }
    }
}

#[derive(Default)]
struct Fields(VecDeque<FieldKey>);
impl Global for Fields {}

/// The asset cache never forgets on its own, and a field is decoded pixels the sweep cannot reach.
fn keep(key: &FieldKey, window: &mut Window, cx: &mut App) {
    let kept = &mut cx.default_global::<Fields>().0;
    if let Some(index) = kept.iter().position(|old| old == key) {
        kept.remove(index);
    }
    kept.push_back(key.clone());
    let expired = (kept.len() > KEPT).then(|| kept.pop_front()).flatten();
    if let Some(expired) = expired {
        if let Some(Some(field)) = window.get_asset::<Field>(&expired, cx) {
            cx.drop_image(field, Some(window));
        }
        cx.remove_asset::<Field>(&expired);
    }
}

/// What `artwork` casts on `ground`; rendering starts on the first call and is shared by every view.
pub fn cast(
    artwork: &ArtworkRef,
    ground: Ground,
    spread: Spread,
    window: &mut Window,
    cx: &mut App,
) -> Cast {
    let key = FieldKey {
        artwork: ArtworkKey::new(artwork, false),
        ground,
        spread,
    };
    keep(&key, window, cx);
    match window.use_asset::<Field>(&key, cx) {
        None => Cast::Pending,
        Some(None) => Cast::Nothing,
        Some(Some(field)) => Cast::Light(field),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba as Pixel};

    const NIGHT: Ground = Ground {
        background: [0, 0, 0],
        ink: [148, 163, 184],
        dark: true,
    };
    const DAY: Ground = Ground {
        background: [250, 250, 250],
        ink: [100, 116, 139],
        dark: false,
    };

    fn encode(pixels: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let image: ImageBuffer<Pixel<u8>, Vec<u8>> =
            ImageBuffer::from_fn(64, 64, |x, y| Pixel(pixels(x, y)));
        let mut bytes = Vec::new();
        image
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .expect("a test image encodes");
        bytes
    }

    fn lattice() -> impl Iterator<Item = (f32, f32)> {
        (0..=20).flat_map(|x| (0..=20).map(move |y| (x as f32 / 20., y as f32 / 20.)))
    }

    #[test]
    fn a_grey_cover_casts_nothing() {
        let grey = encode(|x, _| [(x * 4) as u8, (x * 4) as u8, (x * 4) as u8, 255]);
        assert!(render(&grey, NIGHT, Spread::Full).is_none());
    }

    #[test]
    fn near_black_noise_is_not_mistaken_for_a_hue() {
        // rgb(3, 0, 0) is fully saturated as HSL has it, and says nothing about colour.
        let murk = encode(|_, _| [3, 0, 0, 255]);
        assert!(render(&murk, NIGHT, Spread::Full).is_none());
    }

    #[test]
    fn the_light_pools_at_the_top_and_behind_the_cover_and_leaves_the_middle() {
        let top = strength(0.5, 0., Spread::Glow, NIGHT);
        let lamp = strength(0.02, 0.98, Spread::Glow, NIGHT);
        let middle = strength(0.6, 0.62, Spread::Glow, NIGHT);
        assert!(top > 0.3, "{top}");
        assert!(lamp > 0.25, "{lamp}");
        assert!(middle < 0.03, "{middle}");
    }

    #[test]
    fn the_light_never_passes_its_peak_and_full_never_carries_less_than_glow() {
        for (u, v) in lattice() {
            let (glow, full) = (
                strength(u, v, Spread::Glow, NIGHT),
                strength(u, v, Spread::Full, NIGHT),
            );
            assert!(glow <= PEAK && full <= PEAK_FULL);
            assert!(full >= glow);
            assert!(strength(u, v, Spread::Full, DAY) <= PEAK_FULL * LIGHT_GROUND);
        }
    }

    #[test]
    fn a_red_cover_lights_the_top_red_and_leaves_the_middle_to_the_theme() {
        let red = encode(|_, _| [220, 30, 40, 255]);
        let field = render(&red, NIGHT, Spread::Glow).expect("a red cover casts light");
        let top = field.get_pixel(WIDTH / 2, 0).0;
        assert!(top[0] > top[1] + 20 && top[0] > top[2] + 20, "{top:?}");
        let middle = field.get_pixel(WIDTH * 3 / 4, HEIGHT * 2 / 3).0;
        assert_eq!(middle, [0, 0, 0, 255]);
    }

    #[test]
    fn the_frame_goes_to_the_gpu_with_blue_first() {
        let red = encode(|_, _| [220, 30, 40, 255]);
        let field = render(&red, NIGHT, Spread::Glow).expect("a red cover casts light");
        let top = field.get_pixel(WIDTH / 2, 0).0;
        let framed = frame(field);
        let bytes = framed.as_bytes(0).expect("a frame has bytes");
        let at = (WIDTH as usize / 2) * 4;
        assert_eq!(&bytes[at..at + 4], &[top[2], top[1], top[0], 255]);
    }

    #[test]
    fn the_same_cover_renders_the_same_bytes() {
        let cover = encode(|x, y| [(x * 4) as u8, 40, (y * 4) as u8, 255]);
        let first = render(&cover, NIGHT, Spread::Full).expect("it casts light");
        let second = render(&cover, NIGHT, Spread::Full).expect("it casts light");
        assert_eq!(first.as_raw(), second.as_raw());
    }

    #[test]
    fn the_grain_is_one_level_either_way_and_adds_nothing_on_average() {
        let mut sum = 0.;
        for x in 0..64 {
            for y in 0..64 {
                let noise = grain(x, y);
                assert!((-1. ..=1.).contains(&noise));
                sum += noise;
            }
        }
        assert!((sum / 4096.).abs() < 0.05, "{}", sum / 4096.);
    }

    #[test]
    fn the_faintest_ink_still_reads_over_the_brightest_the_light_gets() {
        let loud = [
            [255, 0, 0],
            [255, 230, 0],
            [0, 255, 120],
            [0, 200, 255],
            [255, 0, 220],
            [255, 255, 255],
        ];
        for spread in [Spread::Glow, Spread::Full] {
            for ground in [NIGHT, DAY] {
                for colour in loud {
                    let cell = tone(colour, ground, spread);
                    let worst = mix(
                        linear_rgb(ground.background),
                        cell.colour,
                        peak(ground, spread) * cell.weight.min(1.),
                    );
                    let ratio = contrast(linear_rgb(ground.ink), worst);
                    assert!(
                        ratio >= READABLE - 0.05,
                        "{colour:?} on {ground:?} at {spread:?}: {ratio}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_cell_between_a_colour_and_a_grey_keeps_the_colours_hue() {
        let red = tone([220, 30, 40], NIGHT, Spread::Glow);
        let grey = tone([128, 128, 128], NIGHT, Spread::Glow);
        let mut grid = vec![grey; CELLS * CELLS];
        grid[0] = red;
        let between = sample(
            &grid,
            &shares(1. / CELLS as f32),
            &shares(0.5 / CELLS as f32),
        );
        assert!(between.weight > 0. && between.weight < red.weight);
        assert!(between.colour[0] > between.colour[1] * 4., "{between:?}");
    }
}
