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
    /// Diagnose dependencies, private file permissions and MusicKit readiness
    Doctor,
    /// Install the local Firefox bridge files and print temporary add-on setup
    FirefoxSetup,
    #[command(hide = true)]
    NativeHost {
        #[arg(trailing_var_arg = true)]
        browser_args: Vec<String>,
    },
    /// Measure native/browser RSS, PSS and sampled CPU as JSON
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
    /// Enter a Music User Token privately; --browser explicitly authorizes the legacy backend
    Login {
        #[arg(long)]
        browser: bool,
    },
    /// Remove the saved local Music User Token; --browser revokes legacy MusicKit authorization
    Logout {
        #[arg(long)]
        browser: bool,
    },
    /// Print current service state as JSON
    Status,
    /// Send a terminal/SSH control: play, pause, toggle, next, previous, mute, seek, volume, shuffle, repeat, sleep, clear, stop
    Ctl {
        action: String,
        value: Option<String>,
    },
    /// Search the real Apple Music catalog through the configured browser
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
        Some(Commands::Login { browser: false }) => rustune::auth::prompt_login(&paths)?,
        Some(Commands::Logout { browser: false }) => {
            rustune::auth::remove_user_token(&paths)?;
            println!(
                "Local Music User Token removed. Browser/account authorization was not changed."
            );
        }
        Some(Commands::NativeHost { browser_args }) => {
            rustune::backend::firefox::native_host(paths, &browser_args).await?
        }
        Some(Commands::FirefoxSetup) => {
            let manifest = rustune::backend::firefox::setup(&paths)?;
            let mut config = config;
            config.backend = rustune::config::BackendKind::Firefox;
            config.save(&paths)?;
            println!(
                "Open about:debugging#/runtime/this-firefox in Firefox → Load Temporary Add-on → {}",
                manifest.display()
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
                    Commands::Login { browser: true } => Request::Login,
                    Commands::Logout { browser: true } => Request::Logout,
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
    println!("Configured backend: {:?}", config.backend);
    println!(
        "Firefox bridge: {}",
        if paths.runtime.join("firefox.sock").exists() {
            "native host socket exists; controller/account acceptance unverified"
        } else {
            "not connected; run rustune firefox-setup and load its add-on"
        }
    );
    println!(
        "Optional Chromium browser: {}",
        rustune::backend::musickit::browser_binary(config)
            .map_or("MISSING".into(), |p| p.display().to_string())
    );
    println!(
        "MusicKit developer token: {}",
        if paths.token().is_ok() {
            "private JWT file present (server acceptance unverified)"
        } else {
            "not configured; optional for Firefox, required for music_kit backend"
        }
    );
    println!(
        "Music User Token: {}",
        if paths.config.join("music-user-token").exists() {
            "local file present; account acceptance not verified; separate from developer JWT"
        } else {
            "managed by official browser sign-in"
        }
    );
    println!("XDG runtime: {}", paths.runtime.display());
    println!(
        "Display: {}",
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            "Wayland"
        } else if std::env::var_os("DISPLAY").is_some() {
            "X11"
        } else {
            "none; browser sign-in needs a graphical session"
        }
    );
    for (name, args) in [("pw-cli", vec!["info", "0"]), ("wpctl", vec!["status"])] {
        let check = tokio::time::timeout(
            Duration::from_secs(3),
            tokio::process::Command::new(name)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .status(),
        )
        .await;
        println!(
            "{name}: {}",
            if matches!(check,Ok(Ok(s)) if s.success()) {
                "available and responding"
            } else {
                "unavailable or not responding"
            }
        );
    }
    println!(
        "Session D-Bus: {}",
        if zbus::Connection::session().await.is_ok() {
            "connected"
        } else {
            "unavailable"
        }
    );
    let widevine = [
        "/usr/lib/chromium/WidevineCdm",
        "/opt/google/chrome/WidevineCdm",
        "/usr/lib/chromium/libwidevinecdm.so",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).exists());
    println!(
        "Widevine: {}",
        if widevine {
            "installation found; actual EME playback still needs verification"
        } else {
            "not found at common paths; verify your browser's protected-content support"
        }
    );
    println!(
        "Audio: Auto only; codec/bitrate/bit-depth unknown; free LRCLIB lyrics enabled; lossless/DSP/visualizer unavailable"
    );
    if let Ok(v) = ipc::request(paths, &Request::Status).await {
        let s: Status = serde_json::from_value(v)?;
        println!(
            "Service: PID {}; browser {:?}; MusicKit authorized {}",
            s.pid, s.browser_pid, s.playback.authorized
        );
    } else {
        println!("Service: not running (launch is lazy)");
    }
    println!(
        "Live full-song playback: NOT VERIFIED by doctor; authorization alone does not establish an active subscription or DRM compatibility."
    );
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
