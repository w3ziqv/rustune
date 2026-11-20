//! MPRIS 2.2 over the Linux session bus; no network listener.
use crate::{
    ipc::Request,
    model::Repeat,
    service::{Sender, Status, send},
};
use std::collections::HashMap;
use tokio::sync::watch;
use zbus::{
    Connection, fdo,
    wire::{OwnedObjectPath, OwnedVariant as OwnedValue, Str},
};
const TRACK_PREFIX: &str = "/io/github/w3ziqv/Rustune/track/";
const PATH: &str = "/org/mpris/MediaPlayer2";
#[derive(Clone)]
struct Root {
    tx: Sender,
}
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    async fn raise(&self) {}
    async fn quit(&self) -> fdo::Result<()> {
        control(&self.tx, Request::Shutdown).await
    }
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &str {
        "Rustune"
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "rustune"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<&str> {
        vec![]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<&str> {
        vec![]
    }
}
struct Player {
    tx: Sender,
    state: watch::Receiver<Status>,
}
async fn control(tx: &Sender, r: Request) -> fdo::Result<()> {
    send(tx, r)
        .await
        .map(|_| ())
        .map_err(|e| fdo::Error::Failed(e.to_string()))
}
fn loop_name(value: Repeat) -> &'static str {
    match value {
        Repeat::Off => "None",
        Repeat::All => "Playlist",
        Repeat::One => "Track",
    }
}
fn track_path(state: &Status) -> OwnedObjectPath {
    let path = state.current_entry_id.map_or_else(
        || "/org/mpris/MediaPlayer2/TrackList/NoTrack".to_string(),
        |id| format!("{TRACK_PREFIX}{id}"),
    );
    OwnedObjectPath::try_from(path).expect("numeric MPRIS track ID")
}
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    async fn next(&self) -> fdo::Result<()> {
        control(&self.tx, Request::MprisNext).await
    }
    async fn previous(&self) -> fdo::Result<()> {
        control(&self.tx, Request::MprisPrevious).await
    }
    async fn pause(&self) -> fdo::Result<()> {
        control(&self.tx, Request::MprisPause).await
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        control(&self.tx, Request::MprisToggle).await
    }
    async fn stop(&self) -> fdo::Result<()> {
        control(&self.tx, Request::StopAudio).await
    }
    async fn play(&self) -> fdo::Result<()> {
        control(&self.tx, Request::MprisPlay).await
    }
    async fn seek(&self, offset: i64) -> fdo::Result<()> {
        control(&self.tx, Request::RelativeSeek(offset)).await
    }
    async fn set_position(&self, track_id: OwnedObjectPath, position: i64) -> fdo::Result<()> {
        let Some(id) = track_id.as_str().strip_prefix(TRACK_PREFIX) else {
            return Ok(());
        };
        let Ok(entry_id) = id.parse::<u64>() else {
            return Ok(());
        };
        if id != entry_id.to_string() {
            return Ok(());
        }
        control(
            &self.tx,
            Request::SetPosition {
                entry_id,
                position_us: position,
            },
        )
        .await
    }
    async fn open_uri(&self, _uri: &str) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported(
            "use catalog search in Rustune".into(),
        ))
    }
    #[zbus(property)]
    fn playback_status(&self) -> &str {
        self.state.borrow().playback_status()
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn loop_status(&self) -> &str {
        loop_name(self.state.borrow().repeat)
    }
    #[zbus(property)]
    async fn set_loop_status(&self, value: &str) -> fdo::Result<()> {
        let desired = match value {
            "None" => Repeat::Off,
            "Playlist" => Repeat::All,
            "Track" => Repeat::One,
            _ => return Err(fdo::Error::InvalidArgs("invalid loop status".into())),
        };
        control(&self.tx, Request::SetRepeat(desired)).await
    }
    #[zbus(property(emits_changed_signal = "const"))]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn set_rate(&self, value: f64) -> fdo::Result<()> {
        if value == 1.0 {
            Ok(())
        } else {
            Err(fdo::Error::NotSupported("playback rate is fixed".into()))
        }
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn shuffle(&self) -> bool {
        self.state.borrow().shuffle
    }
    #[zbus(property)]
    async fn set_shuffle(&self, value: bool) -> fdo::Result<()> {
        control(&self.tx, Request::SetShuffle(value)).await
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let s = self.state.borrow();
        let mut m = HashMap::new();
        if let Some(t) = &s.playback.track {
            if let Ok(v) = OwnedValue::try_from(zbus::wire::Variant::from(track_path(&s))) {
                m.insert("mpris:trackid".into(), v);
            }
            let length = if s.playback.duration > 0.0 {
                crate::model::micros(s.playback.duration)
            } else {
                t.duration_ms.saturating_mul(1000).min(i64::MAX as u64) as i64
            };
            if length > 0 {
                m.insert("mpris:length".into(), OwnedValue::from(length));
            }
            m.insert(
                "xesam:title".into(),
                OwnedValue::from(Str::from(t.title.clone())),
            );
            m.insert(
                "xesam:album".into(),
                OwnedValue::from(Str::from(t.album.clone())),
            );
            if let Ok(v) = OwnedValue::try_from(zbus::wire::Variant::from(vec![t.artist.as_str()]))
            {
                m.insert("xesam:artist".into(), v);
            }
            if let Some(url) = &t.artwork {
                m.insert(
                    "mpris:artUrl".into(),
                    OwnedValue::from(Str::from(url.replace("{w}", "256").replace("{h}", "256"))),
                );
            }
        }
        m
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn volume(&self) -> f64 {
        self.state.borrow().playback.volume
    }
    #[zbus(property)]
    async fn set_volume(&self, v: f64) -> fdo::Result<()> {
        control(&self.tx, Request::Volume(v.max(0.0))).await
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        crate::model::micros(self.state.borrow().playback.position)
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.state.borrow().can_go_next()
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.state.borrow().can_go_previous()
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.state.borrow().can_play()
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.state.borrow().can_pause()
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.state.borrow().can_seek()
    }
    #[zbus(property(emits_changed_signal = "const"))]
    fn can_control(&self) -> bool {
        true
    }
    #[zbus(signal)]
    async fn seeked(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        position: i64,
    ) -> zbus::Result<()>;
}

pub(crate) async fn emit_seeked(connection: &Connection, position: i64) -> zbus::Result<()> {
    let interface = connection
        .object_server()
        .interface::<_, Player>(PATH)
        .await?;
    Player::seeked(interface.signal_emitter(), position).await
}

pub async fn start(
    tx: Sender,
    state: watch::Receiver<Status>,
) -> zbus::Result<(Connection, tokio::task::JoinHandle<()>)> {
    start_at(tx, state, None).await
}
pub(crate) async fn start_at(
    tx: Sender,
    mut state: watch::Receiver<Status>,
    address: Option<&str>,
) -> zbus::Result<(Connection, tokio::task::JoinHandle<()>)> {
    let builder = if let Some(address) = address {
        zbus::connection::Builder::address(address)
    } else {
        zbus::connection::Builder::session()
    };
    let connection = builder
        .name("org.mpris.MediaPlayer2.rustune")
        .serve_at(PATH, Root { tx: tx.clone() })
        .serve_at(
            PATH,
            Player {
                tx,
                state: state.clone(),
            },
        )
        .build()
        .await?;
    let bus = connection.clone();
    let mut old = state.borrow_and_update().clone();
    let task = tokio::spawn(async move {
        while state.changed().await.is_ok() {
            let new = state.borrow_and_update().clone();
            if let Ok(interface) = bus.object_server().interface::<_, Player>(PATH).await {
                let player = interface.get().await;
                let emitter = interface.signal_emitter();
                if old.playback_status() != new.playback_status() {
                    let _ = player.playback_status_changed(emitter).await;
                }
                if old.playback.track != new.playback.track
                    || old.current_entry_id != new.current_entry_id
                    || old.playback.duration != new.playback.duration
                {
                    let _ = player.metadata_changed(emitter).await;
                }
                // Publish from the service snapshot for writes from every entry point.
                // Automatic setter notifications would send a second, possibly stale value.
                let mut changed = HashMap::new();
                if old.playback.volume != new.playback.volume {
                    changed.insert("Volume", new.playback.volume.into());
                }
                if old.shuffle != new.shuffle {
                    changed.insert("Shuffle", new.shuffle.into());
                }
                if old.repeat != new.repeat {
                    changed.insert("LoopStatus", loop_name(new.repeat).into());
                }
                if !changed.is_empty() {
                    let _ = fdo::Properties::properties_changed(
                        emitter,
                        zbus::names::InterfaceName::from_static_str(
                            "org.mpris.MediaPlayer2.Player",
                        )
                        .expect("static interface name"),
                        changed,
                        Default::default(),
                    )
                    .await;
                }
                if old.can_go_next() != new.can_go_next() {
                    let _ = player.can_go_next_changed(emitter).await;
                }
                if old.can_go_previous() != new.can_go_previous() {
                    let _ = player.can_go_previous_changed(emitter).await;
                }
                if old.can_play() != new.can_play() {
                    let _ = player.can_play_changed(emitter).await;
                }
                if old.can_pause() != new.can_pause() {
                    let _ = player.can_pause_changed(emitter).await;
                }
                if old.can_seek() != new.can_seek() {
                    let _ = player.can_seek_changed(emitter).await;
                }
            }
            old = new;
        }
    });
    Ok((connection, task))
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    #[test]
    fn transport_capabilities_follow_active_audio_after_queue_clear_and_eof() {
        for (audio_pid, queue_len, can_play, can_pause) in
            [(Some(2u32), 0, true, true), (None, 2, true, false)]
        {
            let status:Status=serde_json::from_value(serde_json::json!({"playback":{"duration":100.0,"track":{"id":"saved"}},"queue_len":queue_len,"current":null,"repeat":"off","shuffle":false,"profile":"balanced","pid":1,"browser_pid":null,"audio_pid":audio_pid,"start_available":true,"backend":"native","browser_shared":false,"playback_available":true,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap();
            let (tx, _) = tokio::sync::mpsc::channel(1);
            let (_, state) = watch::channel(status);
            let player = Player { tx, state };
            assert_eq!(player.can_play(), can_play);
            assert_eq!(player.can_pause(), can_pause);
            assert_eq!(player.can_seek(), can_pause);
        }
    }
    #[test]
    fn stopped_metadata_and_duplicate_song_occurrences_use_session_state() {
        let status:Status=serde_json::from_value(serde_json::json!({"playback":{"phase":"failed","track":{"id":"same"}},"queue_len":2,"current":0,"current_entry_id":17,"can_next":false,"can_previous":true,"repeat":"all","shuffle":true,"profile":"balanced","pid":1,"browser_pid":null,"start_available":true,"backend":"native","browser_shared":false,"playback_available":true,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null})).unwrap();
        let (tx, _) = tokio::sync::mpsc::channel(1);
        let (_, state) = watch::channel(status.clone());
        let player = Player { tx, state };
        assert_eq!(player.playback_status(), "Stopped");
        assert!(!player.can_go_next());
        assert!(player.can_go_previous());
        let first = track_path(&status);
        let mut other = status;
        other.current_entry_id = Some(18);
        assert_ne!(first, track_path(&other));
    }
    #[test]
    fn disabled_engine_does_not_advertise_restored_queue_as_playable() {
        let status:Status=serde_json::from_value(serde_json::json!({
            "playback":{"duration":100.0,"track":{"id":"saved"}},
            "queue_len":2,"current":1,"repeat":"all","shuffle":true,"profile":"balanced",
            "pid":1,"browser_pid":null,"backend":"disabled","browser_shared":false,
            "playback_available":false,"uptime_secs":0,"cache_entries":0,"message":"","sleep_remaining":null
        })).unwrap();
        let (tx, _) = tokio::sync::mpsc::channel(1);
        let (_, state) = watch::channel(status);
        let player = Player { tx, state };
        assert!(player.can_control());
        assert!(!player.can_play());
        assert!(!player.can_pause());
        assert!(!player.can_seek());
        assert!(!player.can_go_next());
        assert!(!player.can_go_previous());
    }
}

#[cfg(test)]
mod transport_tests;
