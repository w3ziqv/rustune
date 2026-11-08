//! Bounded, lazy artwork loading. No credentials are sent to image hosts.
use crate::{cache::Cache, config::PerformanceMode};
use anyhow::{Result, ensure};
use image::{DynamicImage, ImageReader};
use ratatui::layout::Size;
use ratatui_image::{
    Resize,
    picker::{Picker, ProtocolType},
    protocol::Protocol,
};
use std::{io::Cursor, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

const MAX_BYTES: usize = 2 * 1024 * 1024;
pub const COVER_SIZE: Size = Size::new(24, 12);

pub fn artwork_url(template: &str, pixels: u32) -> Result<reqwest::Url> {
    ensure!(
        template.len() <= 4096 && (1..=512).contains(&pixels),
        "invalid artwork request"
    );
    let url = reqwest::Url::parse(
        &template
            .replace("{w}", &pixels.to_string())
            .replace("{h}", &pixels.to_string()),
    )?;
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.port_or_known_default() == Some(443)
            && url.host_str().is_some_and(|h| h.ends_with(".mzstatic.com")),
        "unsupported artwork host"
    );
    Ok(url)
}

pub fn decode_cover(bytes: &[u8], pixels: u32) -> Result<DynamicImage> {
    ensure!(
        bytes.len() <= MAX_BYTES && (1..=512).contains(&pixels),
        "artwork exceeds limits"
    );
    let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode()?.thumbnail(pixels, pixels))
}

async fn fetch(client: &reqwest::Client, template: &str, pixels: u32) -> Result<DynamicImage> {
    let mut response = client
        .get(artwork_url(template, pixels)?)
        .send()
        .await?
        .error_for_status()?;
    ensure!(
        response.content_length().unwrap_or(0) <= MAX_BYTES as u64,
        "artwork exceeds limits"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= MAX_BYTES,
            "artwork exceeds limits"
        );
        bytes.extend_from_slice(&chunk);
    }
    tokio::task::spawn_blocking(move || decode_cover(&bytes, pixels)).await?
}

#[derive(Clone, PartialEq, Eq)]
pub struct CoverRequest {
    pub size: Size,
    pub url: String,
    pub profile: PerformanceMode,
    pub protocol: String,
    pub background: [u8; 3],
}
pub struct CoverResult {
    pub request: CoverRequest,
    pub image: Option<Arc<Protocol>>,
}

fn encode(image: DynamicImage, request: &CoverRequest) -> Result<Protocol> {
    ensure!(
        (1..=96).contains(&request.size.width) && (1..=48).contains(&request.size.height),
        "artwork cell area exceeds limit"
    );
    let mut picker = Picker::halfblocks();
    let protocol = match request.protocol.as_str() {
        "kitty" => ProtocolType::Kitty,
        "iterm2" => ProtocolType::Iterm2,
        // Halfblocks works without terminal queries and through multiplexers.
        _ => ProtocolType::Halfblocks,
    };
    picker.set_protocol_type(protocol);
    let [r, g, b] = request.background;
    picker.set_background_color(Some(image::Rgba([r, g, b, 255])));
    // Picker fits without upscaling; enlarge to the bounded terminal cell target
    // first so low-resolution Apple covers still fill the selected layout.
    let image = image.resize(
        u32::from(request.size.width) * 10,
        u32::from(request.size.height) * 20,
        image::imageops::FilterType::Nearest,
    );
    Ok(picker.new_protocol(image, request.size, Resize::Fit(None))?)
}

/// At most one fetch/decode runs and one latest request waits. Cache encoded
/// images rather than cloning decoded bitmaps on each render.
pub fn worker() -> (
    watch::Sender<Option<CoverRequest>>,
    mpsc::Receiver<CoverResult>,
    tokio::task::JoinHandle<()>,
) {
    let (requests, mut pending) = watch::channel::<Option<CoverRequest>>(None);
    let (done, results) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        let Ok(client) = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(5))
            .build()
        else {
            return;
        };
        let mut cache = Cache::<CoverRequest, Arc<Protocol>>::new(4);
        while pending.changed().await.is_ok() {
            let request = pending.borrow_and_update().clone();
            let Some(request) = request else {
                cache.entries.clear();
                continue;
            };
            cache.capacity = request.profile.limits().artwork_entries;
            while cache.entries.len() > cache.capacity {
                cache.entries.pop_front();
            }
            if cache.capacity == 0 {
                continue;
            }
            let image = if let Some(image) = cache.get(&request) {
                Some(Arc::clone(image))
            } else {
                match fetch(
                    &client,
                    &request.url,
                    request.profile.limits().artwork_pixels,
                )
                .await
                {
                    Ok(image) => {
                        let options = request.clone();
                        tokio::task::spawn_blocking(move || encode(image, &options))
                            .await
                            .ok()
                            .and_then(Result::ok)
                            .map(Arc::new)
                    }
                    Err(_) => None,
                }
            };
            // A navigation/track change invalidates any result still in flight.
            if pending.has_changed().unwrap_or(true) {
                continue;
            }
            if let Some(image) = &image {
                cache.insert(request.clone(), Arc::clone(image));
            }
            if done.send(CoverResult { request, image }).await.is_err() {
                break;
            }
        }
    });
    (requests, results, task)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cover_encoding_fills_the_requested_cell_area_even_for_small_source_images() {
        let request = CoverRequest {
            size: Size::new(58, 29),
            url: String::new(),
            profile: PerformanceMode::Balanced,
            protocol: "halfblocks".into(),
            background: [0, 0, 0],
        };
        let encoded = encode(DynamicImage::new_rgb8(64, 64), &request).unwrap();
        assert_eq!(encoded.size(), Size::new(58, 29));
    }
}
