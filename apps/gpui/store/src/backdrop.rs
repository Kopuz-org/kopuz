//! A cover blurred for use behind the sheet, keyed like `palette` so one blur serves every view.

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui_kit::{App, Asset, Global, ImageCacheError, ImageSource, ImgResourceLoader, RenderImage};

/// Blurred backdrops kept at once. Small on purpose: these are decoded pixels the sweep cannot reach.
const KEPT: usize = 4;

#[derive(Clone)]
struct BlurKey {
    image: Arc<RenderImage>,
    sigma_hundredths: u32,
}

impl Hash for BlurKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.image.id.hash(state);
        self.sigma_hundredths.hash(state);
    }
}

/// One blurred cover; GPUI's asset cache keys it and dedupes it, as it does the palette.
enum BlurredBackdrop {}

impl Asset for BlurredBackdrop {
    type Source = BlurKey;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    #[expect(clippy::manual_async_fn, reason = "the Send future must not hold App")]
    fn load(source: BlurKey, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        async move { blur_image(&source.image, source.sigma_hundredths) }
    }
}

fn blur_image(
    image: &RenderImage,
    sigma_hundredths: u32,
) -> Result<Arc<RenderImage>, ImageCacheError> {
    let size = image.size(0);
    if size.width.0 <= 0 || size.height.0 <= 0 {
        return Err(ImageCacheError::Asset("empty backdrop image".into()));
    }
    let bytes = image.as_bytes(0).unwrap_or_default().to_vec();
    let pixels = image::RgbaImage::from_raw(size.width.0 as u32, size.height.0 as u32, bytes)
        .ok_or_else(|| ImageCacheError::Asset("invalid backdrop image".into()))?;
    // GPUI stores BGRA; the blur treats each channel independently and preserves that order.
    let blurred = image::imageops::blur(&pixels, sigma_hundredths as f32 / 100.);
    Ok(Arc::new(RenderImage::new(vec![image::Frame::new(blurred)])))
}

#[derive(Default)]
struct Backdrops(VecDeque<BlurKey>);
impl Global for Backdrops {}

/// Blur in displayed pixels, reusing the source decoder and keeping only a few processed backdrops.
pub fn blurred_backdrop(source: ImageSource, radius: u8) -> ImageSource {
    if radius == 0 {
        return source;
    }
    ImageSource::from(move |window: &mut gpui_kit::Window, cx: &mut App| {
        let image = match &source {
            ImageSource::Resource(resource) => window.use_asset::<ImgResourceLoader>(resource, cx),
            ImageSource::Custom(load) => load(window, cx),
            ImageSource::Render(image) => Some(Ok(image.clone())),
            ImageSource::Image(image) => image.clone().use_render_image(window, cx).map(Ok),
        }?;
        let image = match image {
            Ok(image) => image,
            Err(error) => return Some(Err(error)),
        };
        let size = image.size(0);
        let viewport = window.viewport_size();
        let scale = (f32::from(viewport.width) / size.width.0.max(1) as f32)
            .max(f32::from(viewport.height) / size.height.0.max(1) as f32)
            .max(f32::EPSILON);
        let key = BlurKey {
            image,
            sigma_hundredths: (f32::from(radius.min(100)) / scale * 100.).round() as u32,
        };
        let cached = &mut cx.default_global::<Backdrops>().0;
        if let Some(index) = cached.iter().position(|old| {
            old.image.id == key.image.id && old.sigma_hundredths == key.sigma_hundredths
        }) {
            cached.remove(index);
        }
        cached.push_back(key.clone());
        let expired = (cached.len() > KEPT).then(|| cached.pop_front()).flatten();
        if let Some(expired) = expired {
            if let Some(Ok(image)) = window.get_asset::<BlurredBackdrop>(&expired, cx) {
                cx.drop_image(image, Some(window));
            }
            cx.remove_asset::<BlurredBackdrop>(&expired);
        }
        window.use_asset::<BlurredBackdrop>(&key, cx)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blur_softens_edges_without_changing_dimensions_or_channel_order() {
        let pixels = image::RgbaImage::from_fn(21, 5, |x, _| {
            if x < 10 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 0, 255])
            }
        });
        let source = RenderImage::new(vec![image::Frame::new(pixels)]);
        let blurred = blur_image(&source, 200).unwrap();
        assert_eq!(blurred.size(0), source.size(0));
        let pixel = &blurred.as_bytes(0).unwrap()[(2 * 21 + 10) * 4..][..4];
        assert!(pixel[0] > 0 && pixel[0] < 255);
        assert_eq!(&pixel[1..], &[0, 0, 255]);
        assert_eq!(source.as_bytes(0).unwrap()[(2 * 21 + 10) * 4], 0);
    }

    #[test]
    fn zero_blur_keeps_the_original_image_source() {
        let source = Arc::new(RenderImage::new(vec![image::Frame::new(
            image::RgbaImage::new(1, 1),
        )]));
        let ImageSource::Render(result) = blurred_backdrop(source.clone().into(), 0) else {
            panic!("zero blur should not process the source");
        };
        assert!(Arc::ptr_eq(&source, &result));
    }
}
