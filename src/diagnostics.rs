//! Read-only checks for the local native playback installation.
use crate::{
    auth,
    backend::{controller, worker},
    config::Paths,
    ipc::{self, Request},
    service::{Reply, Status},
};
use rustune_core::protocol::PROTOCOL_VERSION;
use serde::Serialize;
use std::{io::ErrorKind, time::Duration};
use tokio::{net::UnixStream, time::timeout};

pub(crate) const UNKNOWN_QUALITY: &str = "Source quality unknown";

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum TokenStorage {
    Missing,
    InvalidFile,
    SavedLocally,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum AudioHelper {
    Missing,
    Compatible { version: u32 },
    Incompatible { version: u32 },
    Unresponsive,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Service {
    NotRunning,
    Unresponsive,
    Running {
        pid: u32,
        helper_protocol: Option<u32>,
    },
}

#[derive(Serialize)]
pub struct Report {
    token: TokenStorage,
    cdm_present: bool,
    audio_helper: AudioHelper,
    audio_helper_scope: &'static str,
    expected_helper_version: u32,
    service: Service,
    session_dbus: bool,
    source_quality: String,
    next_steps: Vec<&'static str>,
}

impl Report {
    pub fn checks(&self) -> [String; 5] {
        let helper = match self.audio_helper {
            AudioHelper::Missing => "missing or not executable".into(),
            AudioHelper::Compatible { version } => format!("compatible (protocol {version})"),
            AudioHelper::Incompatible { version } => format!(
                "incompatible (protocol {version}, expected {})",
                self.expected_helper_version
            ),
            AudioHelper::Unresponsive => "no valid protocol response".into(),
        };
        [
            format!(
                "Music User Token: {}",
                match self.token {
                    TokenStorage::Missing => "missing",
                    TokenStorage::InvalidFile => "private token file unusable",
                    TokenStorage::SavedLocally => "saved locally · account acceptance unverified",
                }
            ),
            format!(
                "CDM file: {}",
                if self.cdm_present {
                    "present"
                } else {
                    "missing"
                },
            ),
            format!("Audio helper: {helper} · sibling of this rustune"),
            match self.service {
                Service::NotRunning => "Service: not running".into(),
                Service::Unresponsive => "Service: no valid IPC response".into(),
                Service::Running {
                    pid,
                    helper_protocol,
                } => {
                    let protocol = match helper_protocol {
                        Some(version) if version != self.expected_helper_version => {
                            format!("{version}, expected {}", self.expected_helper_version)
                        }
                        Some(version) => version.to_string(),
                        None => "unreported".into(),
                    };
                    format!("Service: running (PID {pid}) · helper protocol {protocol}")
                }
            },
            format!(
                "Session D-Bus: {}",
                if self.session_dbus {
                    "connected"
                } else {
                    "unavailable"
                },
            ),
        ]
    }

    pub fn next_steps(&self) -> &[&'static str] {
        &self.next_steps
    }

    pub fn source_quality(&self) -> &str {
        &self.source_quality
    }
}

pub async fn inspect(paths: &Paths) -> Report {
    let token = if auth::has_user_token(paths) {
        TokenStorage::SavedLocally
    } else {
        match std::fs::symlink_metadata(paths.config.join("music-user-token")) {
            Err(error) if error.kind() == ErrorKind::NotFound => TokenStorage::Missing,
            _ => TokenStorage::InvalidFile,
        }
    };
    let cdm_present = paths.cache.join("drm/libwidevinecdm.so").is_file();
    let (audio_helper, (service, status), session_dbus) =
        tokio::join!(audio_helper(), service(paths), session_dbus());
    let mut next_steps = Vec::new();
    match token {
        TokenStorage::Missing => next_steps.push("Save your Music User Token with rustune login."),
        TokenStorage::InvalidFile => next_steps.push(
            "Use rustune login to replace the private token file in the Rustune config directory.",
        ),
        TokenStorage::SavedLocally => {}
    }
    if !cdm_present {
        next_steps.push("Install a compatible CDM in the Rustune cache at drm/libwidevinecdm.so.");
    }
    match audio_helper {
        AudioHelper::Missing => next_steps.push("Install executable rustune-audio beside rustune."),
        AudioHelper::Incompatible { .. } | AudioHelper::Unresponsive => {
            next_steps.push("Reinstall rustune and rustune-audio from the same build.")
        }
        AudioHelper::Compatible { .. } => {}
    }
    match service {
        Service::NotRunning => next_steps.push("Start a service with rustune or rustune daemon."),
        Service::Unresponsive => next_steps.push("Check or restart the local Rustune service."),
        Service::Running {
            helper_protocol, ..
        } if helper_protocol != Some(PROTOCOL_VERSION) => next_steps
            .push("Restart the Rustune service with matching rustune and rustune-audio binaries."),
        Service::Running { .. } => {}
    }
    if !session_dbus {
        next_steps.push("Use a session with D-Bus if desktop media controls are needed.");
    }
    Report {
        token,
        cdm_present,
        audio_helper,
        audio_helper_scope: "frontend_sibling",
        expected_helper_version: PROTOCOL_VERSION,
        service,
        session_dbus,
        source_quality: status
            .map_or_else(|| UNKNOWN_QUALITY.into(), |s| s.playback.quality_label()),
        next_steps,
    }
}

async fn audio_helper() -> AudioHelper {
    let Some(executable) = std::env::current_exe()
        .ok()
        .and_then(|path| worker::audio_executable(&path).ok())
    else {
        return AudioHelper::Missing;
    };

    match controller::probe_protocol(&executable).await {
        Ok(PROTOCOL_VERSION) => AudioHelper::Compatible {
            version: PROTOCOL_VERSION,
        },
        Ok(version) => AudioHelper::Incompatible { version },
        Err(_) => AudioHelper::Unresponsive,
    }
}

const LOCAL_CHECK_TIMEOUT: Duration = Duration::from_secs(1);

async fn service(paths: &Paths) -> (Service, Option<Status>) {
    let operation = async {
        let mut stream = match UnixStream::connect(paths.socket()).await {
            Ok(stream) => stream,
            Err(error) => {
                return if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::ConnectionRefused
                ) {
                    (Service::NotRunning, None)
                } else {
                    (Service::Unresponsive, None)
                };
            }
        };

        let status = async {
            ipc::write_frame(&mut stream, &Request::Status).await?;
            let frame = ipc::read_frame(&mut stream).await?;
            let reply: Reply = serde_json::from_slice(&frame)?;
            Ok::<Status, anyhow::Error>(serde_json::from_value(reply.value()?)?)
        }
        .await;
        match status {
            Ok(status) => (
                Service::Running {
                    pid: status.pid,
                    helper_protocol: status.helper_protocol,
                },
                Some(status),
            ),
            Err(_) => (Service::Unresponsive, None),
        }
    };
    timeout(LOCAL_CHECK_TIMEOUT, operation)
        .await
        .unwrap_or((Service::Unresponsive, None))
}

async fn session_dbus() -> bool {
    timeout(LOCAL_CHECK_TIMEOUT, zbus::Connection::session())
        .await
        .is_ok_and(|result| result.is_ok())
}
