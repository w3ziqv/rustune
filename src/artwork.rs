//! Bounded, lazy artwork loading. No credentials are sent to image hosts.
use crate::config::PerformanceMode;
use anyhow::{Result, ensure};
use image::{DynamicImage, ImageReader};
use ratatui::layout::Size;
use ratatui_image::protocol::Protocol;
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
    pub cell: (u16, u16),
    pub image_id: u32,
    pub background: [u8; 3],
}
pub struct CoverResult {
    pub request: CoverRequest,
    pub image: Option<Arc<Protocol>>,
}

fn encode(image: &DynamicImage, request: &CoverRequest) -> Result<Protocol> {
    ensure!(
        (1..=96).contains(&request.size.width) && (1..=48).contains(&request.size.height),
        "artwork cell area exceeds limit"
    );
    if request.protocol == "halfblocks" {
        let image = image.resize_exact(
            u32::from(request.size.width),
            u32::from(request.size.height) * 2,
            image::imageops::FilterType::Nearest,
        );
        return Ok(Protocol::Halfblocks(
            ratatui_image::protocol::halfblocks::Halfblocks::new(image, request.size)?,
        ));
    }
    let (cw, ch) = request.cell;
    ensure!(
        (1..=64).contains(&cw) && (1..=128).contains(&ch),
        "invalid cell geometry"
    );
    let width = u32::from(request.size.width) * u32::from(cw);
    let height = u32::from(request.size.height) * u32::from(ch);
    ensure!(
        width as u64 * height as u64 <= 2_000_000,
        "artwork pixel area exceeds limit"
    );
    let filter = if request.protocol == "halfblocks" {
        image::imageops::FilterType::Nearest
    } else {
        image::imageops::FilterType::Triangle
    };
    let resized = image.resize(width, height, filter);
    let [r, g, b] = request.background;
    let mut canvas = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        width,
        height,
        image::Rgba([r, g, b, 255]),
    ));
    image::imageops::overlay(
        &mut canvas,
        &resized,
        i64::from((width - resized.width()) / 2),
        i64::from((height - resized.height()) / 2),
    );
    match request.protocol.as_str() {
        "kitty" => Ok(Protocol::Kitty(ratatui_image::protocol::kitty::Kitty::new(
            canvas,
            request.size,
            request.image_id,
            false,
            false,
        )?)),
        "iterm2" => Ok(Protocol::ITerm2(
            ratatui_image::protocol::iterm2::Iterm2::new(canvas, request.size, false)?,
        )),
        _ => anyhow::bail!("unsupported artwork protocol"),
    }
}

/// At most one fetch/decode runs and one latest request waits. Cache encoded
/// source bitmap for resize; only the UI retains the single encoded result.
pub fn worker() -> (
    watch::Sender<Option<CoverRequest>>,
    mpsc::Receiver<CoverResult>,
    tokio::task::JoinHandle<()>,
) {
    let (requests, mut pending) = watch::channel::<Option<CoverRequest>>(None);
    let (done, results) = mpsc::channel(1);
    let task = tokio::spawn(async move {
        let mut client = None;
        let mut source: Option<(String, u32, Arc<DynamicImage>)> = None;
        while pending.changed().await.is_ok() {
            let request = pending.borrow_and_update().clone();
            let Some(request) = request else {
                source = None;
                continue;
            };
            if request.profile.limits().artwork_entries == 0 {
                source = None;
                continue;
            }
            let pixels = request.profile.limits().artwork_pixels;
            if !source
                .as_ref()
                .is_some_and(|(url, p, _)| url == &request.url && *p == pixels)
            {
                source = None;
                if client.is_none() {
                    client = reqwest::Client::builder()
                        .https_only(true)
                        .redirect(reqwest::redirect::Policy::none())
                        .timeout(Duration::from_secs(5))
                        .build()
                        .ok();
                }
                if let Some(client) = &client
                    && let Ok(image) = fetch(client, &request.url, pixels).await
                {
                    source = Some((request.url.clone(), pixels, Arc::new(image)));
                }
            }
            if pending.has_changed().unwrap_or(true) {
                continue;
            }
            let image = if let Some((_, _, decoded)) = &source {
                let decoded = Arc::clone(decoded);
                let options = request.clone();
                tokio::task::spawn_blocking(move || encode(&decoded, &options))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .map(Arc::new)
            } else {
                None
            };
            // A navigation/track change invalidates any result still in flight.
            if pending.has_changed().unwrap_or(true) {
                continue;
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
            cell: (10, 20),
            image_id: 1,
            background: [0, 0, 0],
        };
        let encoded = encode(&DynamicImage::new_rgb8(64, 64), &request).unwrap();
        assert_eq!(encoded.size(), Size::new(58, 29));
    }
}
