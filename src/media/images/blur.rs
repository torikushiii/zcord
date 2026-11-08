use super::*;

const BLUR_SOURCE: u32 = 64;
const BLUR_SIGMA: f32 = 5.;
const BLUR_MIN_TEXTURE: u32 = 384;
const BLUR_CACHE_LIMIT: usize = 64;
type BlurKey = (Arc<Path>, u32, u32, u32);

#[derive(Default)]
pub(super) struct BlurCache {
    entries: HashMap<BlurKey, Option<Arc<gpui::RenderImage>>>,
    order: VecDeque<BlurKey>,
}

thread_local! {
    pub(super) static BLURRED: RefCell<BlurCache> = RefCell::new(BlurCache::default());
}

impl BlurCache {
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
    }
}

pub fn blurred(spec: &ImageSpec, cx: &App) -> Option<Arc<gpui::RenderImage>> {
    blurred_with(spec, BLUR_SOURCE, BLUR_SIGMA, 1., cx)
}

pub fn blurred_with(
    spec: &ImageSpec,
    source: u32,
    sigma: f32,
    saturation: f32,
    cx: &App,
) -> Option<Arc<gpui::RenderImage>> {
    blurred_at_width(spec, source, 0, sigma, saturation, cx)
}

pub fn blurred_at_width(
    spec: &ImageSpec,
    source: u32,
    width: u32,
    sigma: f32,
    saturation: f32,
    cx: &App,
) -> Option<Arc<gpui::RenderImage>> {
    let path = resolve(spec, source, cx)?;
    let key = (path, sigma.to_bits(), saturation.to_bits(), width);
    BLURRED.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(image) = cache.entries.get(&key) {
            return image.clone();
        }
        if cache.entries.len() >= BLUR_CACHE_LIMIT
            && let Some(oldest) = cache.order.pop_front()
        {
            cache.entries.remove(&oldest);
        }
        let image = blur_file(&key.0, width, sigma, saturation);
        cache.order.push_back(key.clone());
        cache.entries.insert(key, image.clone());
        image
    })
}

fn blur_file(
    path: &Path,
    width: u32,
    sigma: f32,
    saturation: f32,
) -> Option<Arc<gpui::RenderImage>> {
    let bytes = std::fs::read(path).ok()?;
    let mut image = image::load_from_memory(&bytes).ok()?.to_rgba8();
    if width > 0 && image.width() > 0 {
        let height =
            (u64::from(image.height()) * u64::from(width) / u64::from(image.width())).max(1) as u32;
        image =
            image::imageops::resize(&image, width, height, image::imageops::FilterType::Triangle);
    }
    let mut blurred = if sigma > 0. {
        image::imageops::fast_blur(&image, sigma)
    } else {
        image
    };
    for pixel in blurred.pixels_mut() {
        let [r, g, b, _] = pixel.0.map(f32::from);
        let gray = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let saturate = |channel: f32| (gray + (channel - gray) * saturation).round() as u8;
        pixel.0 = [saturate(b), saturate(g), saturate(r), 255];
    }
    let scale = (BLUR_MIN_TEXTURE / blurred.width().max(blurred.height()).max(1)).max(1);
    let texture = image::imageops::resize(
        &blurred,
        blurred.width() * scale,
        blurred.height() * scale,
        image::imageops::FilterType::Triangle,
    );
    Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
        texture,
    )])))
}
