//! The colours a cover is made of, pulled off the UI thread and keyed by `ArtworkRef::version`.

use api::ArtworkRef;
use gpui_kit::{App, Asset, Hsla, Rgba, Window};

use crate::artwork::{ArtworkKey, thumbnail_bytes};

/// A cover reduced to the two colours the chrome borrows: what it is mostly made of, and its liveliest.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swatches {
    pub dominant: Hsla,
    pub vibrant: Hsla,
}

/// The extracted palette of one cover; GPUI's asset cache keys it, dedupes it and keeps it.
pub enum CoverPalette {}

impl Asset for CoverPalette {
    type Source = ArtworkKey;
    type Output = Option<Swatches>;

    fn load(
        source: Self::Source,
        cx: &mut App,
    ) -> impl Future<Output = Self::Output> + Send + 'static {
        let bytes = thumbnail_bytes(&source, cx);
        async move {
            let bytes = bytes.await?;
            let span = tracing::debug_span!("palette.extract");
            let _entered = span.enter();
            extract(&bytes)
        }
    }
}

/// The playing cover's colours, or `None` while they are still being pulled.
pub fn of(artwork: &ArtworkRef, window: &mut Window, cx: &mut App) -> Option<Swatches> {
    window
        .use_asset::<CoverPalette>(&ArtworkKey::new(artwork, false), cx)
        .flatten()
}

/// Start pulling a cover's colours without waiting for them, so they are in hand once it plays.
pub fn warm(artwork: &ArtworkRef, cx: &mut App) {
    let _ = cx.fetch_asset::<CoverPalette>(&ArtworkKey::new(artwork, false));
}

/// The thumbnail is resampled to this square before it is counted; a cover's colours survive it.
const SAMPLE: u32 = 48;

/// Four bits a channel, which is coarse enough that a gradient lands in one bucket.
const BITS: u32 = 4;
const LEVELS: usize = 1 << BITS;
const BUCKETS: usize = LEVELS * LEVELS * LEVELS;

/// Below this a pixel is too dark or too pale to say anything about the cover's colour.
const FLOOR: f32 = 0.06;
const CEILING: f32 = 0.96;

/// Android's palette weights: liveliness matters, but a colour nobody can see does not.
const WEIGHT_SATURATION: f32 = 0.24;
const WEIGHT_LIGHTNESS: f32 = 0.52;
const WEIGHT_POPULATION: f32 = 0.24;

/// A vibrant swatch has to be at least this colourful, else the cover is grey and the dominant stands in.
const VIVID: f32 = 0.3;

/// The dominant and vibrant colours of an encoded cover; the whole of the work happens off the UI thread.
fn extract(bytes: &[u8]) -> Option<Swatches> {
    let decoded = image::load_from_memory(bytes).ok()?;
    let sampled = decoded.thumbnail(SAMPLE, SAMPLE).to_rgba8();
    let mut counts = vec![0u32; BUCKETS];
    let mut sums = vec![[0u32; 3]; BUCKETS];
    let mut counted = 0u32;
    for pixel in sampled.pixels() {
        let [r, g, b, a] = pixel.0;
        if a < 128 {
            continue;
        }
        let bucket = index(r, g, b);
        counts[bucket] += 1;
        let sum = &mut sums[bucket];
        sum[0] += r as u32;
        sum[1] += g as u32;
        sum[2] += b as u32;
        counted += 1;
    }
    if counted == 0 {
        return None;
    }
    let average = |bucket: usize| {
        let count = counts[bucket].max(1);
        rgb(
            (sums[bucket][0] / count) as u8,
            (sums[bucket][1] / count) as u8,
            (sums[bucket][2] / count) as u8,
        )
    };
    let busiest = (0..BUCKETS).max_by_key(|bucket| counts[*bucket])?;
    let dominant = average(busiest);
    let most = counts[busiest] as f32;
    let vibrant = (0..BUCKETS)
        .filter(|bucket| counts[*bucket] > 0)
        .map(|bucket| (bucket, average(bucket)))
        .filter(|(_, color)| color.s >= VIVID && color.l > FLOOR && color.l < CEILING)
        .max_by(|(left, a), (right, b)| {
            let score = |bucket: usize, color: &Hsla| {
                WEIGHT_SATURATION * color.s
                    + WEIGHT_LIGHTNESS * (1. - (color.l - 0.5).abs() * 2.)
                    + WEIGHT_POPULATION * (counts[bucket] as f32 / most)
            };
            score(*left, a).total_cmp(&score(*right, b))
        })
        .map(|(_, color)| color)
        .unwrap_or(dominant);
    Some(Swatches { dominant, vibrant })
}

fn index(r: u8, g: u8, b: u8) -> usize {
    let level = |channel: u8| (channel as usize) >> (8 - BITS);
    (level(r) * LEVELS + level(g)) * LEVELS + level(b)
}

fn rgb(r: u8, g: u8, b: u8) -> Hsla {
    Rgba {
        r: r as f32 / 255.,
        g: g as f32 / 255.,
        b: b as f32 / 255.,
        a: 1.,
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba as Pixel};

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

    fn near(color: Hsla, expected: Hsla) -> bool {
        let (left, right): (Rgba, Rgba) = (color.into(), expected.into());
        (left.r - right.r).abs() < 0.06
            && (left.g - right.g).abs() < 0.06
            && (left.b - right.b).abs() < 0.06
    }

    #[test]
    fn a_flat_picture_is_its_own_dominant_colour() {
        let bytes = encode(|_, _| [0xd9, 0x84, 0x2f, 0xff]);
        let swatches = extract(&bytes).expect("a flat picture has a palette");
        assert!(
            near(swatches.dominant, rgb(0xd9, 0x84, 0x2f)),
            "{:?}",
            swatches.dominant
        );
        assert!(near(swatches.vibrant, swatches.dominant));
    }

    #[test]
    fn the_majority_colour_wins_the_dominant_slot() {
        let bytes = encode(|_, y| {
            if y < 56 {
                [0x10, 0x10, 0x14, 0xff]
            } else {
                [0xe0, 0x30, 0x30, 0xff]
            }
        });
        let swatches = extract(&bytes).expect("a two-tone picture has a palette");
        assert!(
            near(swatches.dominant, rgb(0x10, 0x10, 0x14)),
            "{:?}",
            swatches.dominant
        );
    }

    #[test]
    fn the_vibrant_slot_skips_the_dark_majority_for_the_colourful_minority() {
        let bytes = encode(|_, y| {
            if y < 56 {
                [0x10, 0x10, 0x14, 0xff]
            } else {
                [0xe0, 0x30, 0x30, 0xff]
            }
        });
        let swatches = extract(&bytes).expect("a two-tone picture has a palette");
        assert!(
            near(swatches.vibrant, rgb(0xe0, 0x30, 0x30)),
            "{:?}",
            swatches.vibrant
        );
    }

    #[test]
    fn a_grey_picture_falls_back_to_its_dominant_rather_than_inventing_a_hue() {
        let bytes = encode(|_, _| [0x80, 0x80, 0x80, 0xff]);
        let swatches = extract(&bytes).expect("a grey picture has a palette");
        assert_eq!(swatches.vibrant, swatches.dominant);
        assert!(swatches.vibrant.s < 0.05, "{:?}", swatches.vibrant);
    }

    #[test]
    fn a_fully_transparent_picture_has_nothing_to_extract() {
        let bytes = encode(|_, _| [0xff, 0x00, 0x00, 0x00]);
        assert_eq!(extract(&bytes), None);
    }

    #[test]
    fn rubbish_bytes_do_not_panic() {
        assert_eq!(extract(b"not a picture at all"), None);
    }
}
