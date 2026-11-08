#![cfg_attr(not(target_os = "linux"), allow(unused))]
#[cfg(not(target_os = "linux"))]
compile_error!("Rustune supports Linux exclusively");
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rustune::{
    config::{Config, Paths},
    ipc::{self, Request},
    service::Status,
};
use std::{
    process::Stdio,
    time::{Duration, Instant},
};

#[derive(Parser)]
#[command(
    version,
    about = "A native Linux terminal controller for Apple Music (experimental)"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand)]
enum Commands {
    #[command(hide = true)]
    AudioWorker,
    /// Diagnose private token storage and native playback capabilities
    Doctor,
    /// Measure application RSS, PSS and sampled CPU as JSON
    Stats {
        #[arg(long, default_value_t = 1)]
        seconds: u64,
    },
    /// Measure process launch, row formatting, and a live service interval
    Benchmark {
        #[arg(long, default_value_t = 5)]
        seconds: u64,
    },
    /// Run the private playback service in the foreground
    Daemon,
    /// Enter a Music User Token with masked feedback and store it privately
    Login,
    /// Remove the saved local Music User Token
    Logout,
    /// Print current service state as JSON
    Status,
    /// Send a terminal/SSH control: play, pause, toggle, next, previous, mute, seek, volume, shuffle, repeat, sleep, clear, stop
    Ctl {
        action: String,
        value: Option<String>,
    },
    /// Search the public Apple catalog (songs, albums or artists; no token required)
    Search {
        term: String,
        #[arg(long, default_value = "songs")]
        kind: String,
    },
    /// Browse library, albums, artists, playlists or recent
    Browse {
        #[arg(default_value = "library")]
        kind: String,
    },
    /// Write a default config if none exists
    Init,
    /// Render a real UI screen to plain text (for documentation/terminal checks)
    Render {
        #[arg(long, default_value_t = 100)]
        width: u16,
        #[arg(long, default_value_t = 30)]
        height: u16,
        #[arg(long, default_value_t = 0)]
        screen: usize,
    },
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::discover()?;
    let config = Config::load(&paths)?;
    match cli.command {
        Some(Commands::AudioWorker) => rustune::backend::worker::run(&paths).await?,
        Some(Commands::Login) => rustune::auth::prompt_login(&paths)?,
        Some(Commands::Logout) => {
            rustune::auth::remove_user_token(&paths)?;
            println!(
                "Local Music User Token removed. Apple account authorization was not changed."
            );
        }
        Some(Commands::Doctor) => doctor(&config, &paths).await?,
        Some(Commands::Stats { seconds }) => {
            anyhow::ensure!((1..=60).contains(&seconds), "seconds must be 1..60");
            let status = ipc::request(&paths, &Request::Status)
                .await
                .ok()
                .and_then(|v| serde_json::from_value::<Status>(v).ok());
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &rustune::metrics::sample(status.as_ref(), Duration::from_secs(seconds)).await
                )?
            );
        }
        Some(Commands::Benchmark { seconds }) => benchmark(&paths, seconds).await?,
        Some(Commands::Daemon) => {
            if config.logging {
                tracing_subscriber::fmt()
                    .with_target(false)
                    .with_writer(std::io::stderr)
                    .init();
            }
            rustune::service::run(config, paths).await?;
        }
        Some(Commands::Init) => {
            paths.init()?;
            let p = paths.config.join("config.toml");
            anyhow::ensure!(!p.exists(), "config already exists; preserved");
            config.save(&paths)?;
            println!("Created {}", p.display());
        }
        Some(Commands::Render {
            width,
            height,
            screen,
        }) => {
            anyhow::ensure!(
                width <= 500 && height <= 200,
                "render size limit is 500x200"
            );
            let mut app = rustune::ui::App::new(config);
            app.screen = screen.min(rustune::ui::SCREENS.len() - 1);
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))?;
            terminal.draw(|f| app.render(f))?;
            let b = terminal.backend().buffer();
            for y in 0..height {
                let line: String = (0..width).map(|x| b[(x, y)].symbol()).collect();
                println!("{}", line.trim_end());
            }
        }
        command => {
            let mut child = ensure_service(&paths).await?;
            if let Some(command) = command {
                let request = match command {
                    Commands::Status => Request::Status,
                    Commands::Ctl { action, value } => control(&action, value.as_deref())?,
                    Commands::Search { term, kind } => Request::Browse {
                        kind: format!("search:{kind}"),
                        term,
                        next: None,
                    },
                    Commands::Browse { kind } => Request::Browse {
                        kind,
                        term: String::new(),
                        next: None,
                    },
                    _ => unreachable!("handled above"),
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&ipc::request(&paths, &request).await?)?
                );
            } else {
                let result = rustune::ui::run(config.clone(), paths.clone()).await;
                if child.is_some() && !config.daemon {
                    let _ = ipc::request(&paths, &Request::Stop).await;
                    if let Some(c) = child.as_mut() {
                        let _ = tokio::time::timeout(Duration::from_secs(5), c.wait()).await;
                    }
                }
                result?;
            }
        }
    }
    Ok(())
}
fn control(action: &str, value: Option<&str>) -> Result<Request> {
    let number = || {
        value
            .context("this action needs a numeric value")?
            .parse::<f64>()
            .context("invalid number")
    };
    Ok(match action {
        "play" => Request::Play,
        "pause" => Request::Pause,
        "toggle" => Request::Toggle,
        "next" => Request::Next,
        "previous" => Request::Previous,
        "mute" => Request::Mute,
        "shuffle" => Request::Shuffle,
        "repeat" => Request::Repeat,
        "stop" => Request::Stop,
        "clear" => Request::Clear,
        "seek" => Request::Seek(number()?),
        "volume" => Request::Volume(number()?),
        "sleep" => Request::Sleep(
            value
                .context("sleep requires seconds (0 cancels)")?
                .parse()?,
        ),
        _ => anyhow::bail!("unknown control action; see rustune ctl --help"),
    })
}
async fn ensure_service(paths: &Paths) -> Result<Option<tokio::process::Child>> {
    if ipc::request(paths, &Request::Status).await.is_ok() {
        return Ok(None);
    }
    paths.init()?;
    let mut child = tokio::process::Command::new(std::env::current_exe()?)
        .arg("daemon")
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    for _ in 0..40 {
        if ipc::request(paths, &Request::Status).await.is_ok() {
            return Ok(Some(child));
        }
        if let Some(s) = child.try_wait()? {
            anyhow::bail!("service exited ({s}); run rustune daemon to see the error");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let _ = child.kill().await;
    anyhow::bail!("service failed to start; run rustune daemon for diagnostics")
}
async fn doctor(config: &Config, paths: &Paths) -> Result<()> {
    println!(
        "Rustune {} • Linux {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH
    );
    println!("Profile: {:?}", config.performance);
    println!(
        "Music User Token: {}",
        if rustune::auth::has_user_token(paths) {
            "private local file present; Apple account acceptance NOT VERIFIED"
        } else {
            "not saved; run rustune login"
        }
    );
    println!(
        "Native playback: CDM → AAC → ALSA; {}",
        if rustune::backend::worker::available(paths) {
            "token and CDM files present; live authorization is checked on Play"
        } else {
            "save a token and install a compatible CDM at ~/.cache/rustune/drm/libwidevinecdm.so"
        }
    );
    println!("Audio source quality: unknown");
    println!(
        "Public catalog: Apple Search/Lookup metadata, country {} (no token required)",
        config.catalog_country
    );
    println!("Personal library/playlists: account authorization not implemented");
    println!("Lyrics: free LRCLIB; Artwork: public Apple cover URLs");
    println!("XDG runtime: {}", paths.runtime.display());
    println!(
        "Session D-Bus: {}",
        if zbus::Connection::session().await.is_ok() {
            "connected"
        } else {
            "unavailable"
        }
    );
    if let Ok(v) = ipc::request(paths, &Request::Status).await {
        let status: Status = serde_json::from_value(v)?;
        println!("Service: PID {}; {}", status.pid, status.backend);
    } else {
        println!("Service: not running");
    }
    Ok(())
}
async fn benchmark(paths: &Paths, seconds: u64) -> Result<()> {
    anyhow::ensure!((1..=3600).contains(&seconds), "seconds must be 1..3600");
    let binary = std::env::current_exe()?;
    let mut launches = Vec::new();
    for _ in 0..11 {
        let t = Instant::now();
        let s = tokio::process::Command::new(&binary)
            .arg("--version")
            .stdout(Stdio::null())
            .status()
            .await?;
        anyhow::ensure!(s.success(), "launch benchmark failed");
        launches.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let tracks: Vec<_> = (0..10_000)
        .map(|i| format!("Track {i} — Artist {i}"))
        .collect();
    let mut measured = Vec::new();
    for visible in [false, true] {
        let t = Instant::now();
        for _ in 0..1000 {
            let range = if visible {
                rustune::ui::visible_range(9000, tracks.len(), 20)
            } else {
                0..tracks.len()
            };
            let rows: Vec<_> = range.map(|i| format!("{} {}", i, tracks[i])).collect();
            std::hint::black_box(rows);
        }
        measured.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    let status = ipc::request(paths, &Request::Status)
        .await
        .ok()
        .and_then(|v| serde_json::from_value::<Status>(v).ok());
    let before = rustune::metrics::sample(status.as_ref(), Duration::from_millis(250)).await;
    let report = rustune::metrics::sample(status.as_ref(), Duration::from_secs(seconds)).await;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "launch_first_ms":launches[0],"launch_warm_mean_ms":launches[1..].iter().sum::<f64>()/10.0,
            "cold_start_note":"first launch does not evict Linux page cache; true cold startup is not established",
            "row_formatting_1000_iterations_all_10000_ms":measured[0],"row_formatting_1000_iterations_visible_20_ms":measured[1],
            "row_formatting_note":"synthetic list formatting comparison; not streaming latency or end-to-end navigation latency",
            "initial_total_pss_kib":before.total.pss_kib,"final_measurement":report,
            "playback_verified":false,"active_playback_ram":"unavailable unless you start real playback before benchmarking",
            "long_session_note":"use --seconds 3600 during actual playback; authentication/DRM requires a signed-in subscribed account"
        }))?
    );
    Ok(())
}
