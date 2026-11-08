use rustune::artwork::{artwork_url, decode_cover};
#[test]
fn cover_fetches_are_restricted_to_apples_artwork_hosts() {
    let u = artwork_url(
        "https://is1-ssl.mzstatic.com/image/cover/{w}x{h}bb.jpg",
        256,
    )
    .unwrap();
    assert!(u.path().contains("256x256"));
    for bad in [
        "http://is1-ssl.mzstatic.com/x",
        "https://127.0.0.1/x",
        "https://evil.test/x",
        "https://u:p@is1-ssl.mzstatic.com/x",
        "https://is1-ssl.mzstatic.com:9999/x",
    ] {
        assert!(artwork_url(bad, 256).is_err(), "{bad}");
    }
}
#[test]
fn decoded_covers_are_resized_and_bad_images_fail() {
    let image = image::DynamicImage::new_rgb8(800, 800);
    let mut bytes = std::io::Cursor::new(Vec::new());
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    let decoded = decode_cover(bytes.get_ref(), 128).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (128, 128));
    assert!(decode_cover(b"not an image", 256).is_err());
}

#[tokio::test]
#[ignore = "downloads public Apple artwork; no account, browser or audio playback"]
async fn real_artwork_worker_downloads_and_renders_an_apple_cover() {
    use rustune::{
        artwork::{CoverRequest, worker},
        config::PerformanceMode,
    };
    let (jobs, mut done, task) = worker();
    jobs.send(Some(CoverRequest{
        url:"https://is1-ssl.mzstatic.com/image/thumb/Music112/v4/a9/52/4d/a9524d7e-631b-ece3-cf70-cf37023e8251/886443710016.jpg/{w}x{h}bb.jpg".into(),
        profile:PerformanceMode::Balanced,protocol:"halfblocks".into(),background:[0,0,0],
    })).unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), done.recv())
        .await
        .unwrap()
        .unwrap();
    let image = result
        .image
        .expect("real artwork download/decode/encode failed");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(24, 12)).unwrap();
    terminal
        .draw(|f| f.render_widget(ratatui_image::Image::new(&image), f.area()))
        .unwrap();
    assert!(
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .any(|c| c.symbol() == "▀")
    );
    task.abort();
}
