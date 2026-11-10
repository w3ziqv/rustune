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
fn track_path(state: &Status) -> OwnedObjectPath {
    let id = state
        .current_entry_id
        .map_or("none".into(), |id| id.to_string());
    OwnedObjectPath::try_from(format!("/org/mpris/MediaPlayer2/track/{id}"))
        .expect("sanitized MPRIS object path")
}
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    async fn next(&self) -> fdo::Result<()> {
        control(&self.tx, Request::Next).await
    }
    async fn previous(&self) -> fdo::Result<()> {
        control(&self.tx, Request::Previous).await
    }
    async fn pause(&self) -> fdo::Result<()> {
        control(&self.tx, Request::Pause).await
    }
    async fn play_pause(&self) -> fdo::Result<()> {
        control(&self.tx, Request::Toggle).await
    }
    async fn stop(&self) -> fdo::Result<()> {
        control(&self.tx, Request::StopAudio).await
    }
    async fn play(&self) -> fdo::Result<()> {
        control(&self.tx, Request::Play).await
    }
    async fn seek(&self, offset: i64) -> fdo::Result<()> {
        let v = {
            let state = self.state.borrow();
            let p = &state.playback;
            (p.position + offset as f64 / 1_000_000.0).clamp(0.0, p.duration)
        };
        control(&self.tx, Request::Seek(v)).await
    }
    async fn set_position(&self, track_id: OwnedObjectPath, position: i64) -> fdo::Result<()> {
        let state = self.state.borrow().clone();
        if position < 0 || track_id != track_path(&state) {
            return Ok(());
        }
        control(
            &self.tx,
            Request::Seek((position as f64 / 1_000_000.0).min(state.playback.duration)),
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
        let s = self.state.borrow();
        if s.playback.playing {
            "Playing"
        } else if s.playback.phase == crate::model::PlaybackPhase::Paused {
            "Paused"
        } else {
            "Stopped"
        }
    }
    #[zbus(property)]
    fn loop_status(&self) -> &str {
        match self.state.borrow().repeat {
            Repeat::Off => "None",
            Repeat::All => "Playlist",
            Repeat::One => "Track",
        }
    }
    #[zbus(property)]
    async fn set_loop_status(&self, value: &str) -> fdo::Result<()> {
        let desired = match value {
            "None" => Repeat::Off,
            "Playlist" => Repeat::All,
            "Track" => Repeat::One,
            _ => return Err(fdo::Error::InvalidArgs("invalid loop status".into())),
        };
        let mut current = self.state.borrow().repeat;
        for _ in 0..3 {
            if current == desired {
                break;
            }
            control(&self.tx, Request::Repeat).await?;
            current = match current {
                Repeat::Off => Repeat::All,
                Repeat::All => Repeat::One,
                Repeat::One => Repeat::Off,
            };
        }
        Ok(())
    }
    #[zbus(property)]
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
    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.state.borrow().shuffle
    }
    #[zbus(property)]
    async fn set_shuffle(&self, value: bool) -> fdo::Result<()> {
        let current = self.state.borrow().shuffle;
        if value != current {
            control(&self.tx, Request::Shuffle).await?;
        }
        Ok(())
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let s = self.state.borrow();
        let mut m = HashMap::new();
        if let Some(t) = &s.playback.track {
            if let Ok(v) = OwnedValue::try_from(zbus::wire::Variant::from(track_path(&s))) {
                m.insert("mpris:trackid".into(), v);
            }
            m.insert(
                "mpris:length".into(),
                OwnedValue::from(t.duration_ms.saturating_mul(1000).min(i64::MAX as u64) as i64),
            );
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
    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.state.borrow().playback.volume
    }
    #[zbus(property)]
    async fn set_volume(&self, v: f64) -> fdo::Result<()> {
        control(&self.tx, Request::Volume(v)).await
    }
    #[zbus(property)]
    fn position(&self) -> i64 {
        (self.state.borrow().playback.position * 1_000_000.0) as i64
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
        let s = self.state.borrow();
        s.start_available && s.can_next
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        let s = self.state.borrow();
        s.start_available && s.can_previous
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        let s = self.state.borrow();
        s.audio_pid.is_some() || (s.start_available && s.queue_len > 0)
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        let s = self.state.borrow();
        s.audio_pid.is_some() && s.playback.track.is_some()
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        let s = self.state.borrow();
        s.audio_pid.is_some() && s.playback.duration > 0.0
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        self.state.borrow().playback_available
    }
    #[zbus(signal)]
    async fn seeked(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        position: i64,
    ) -> zbus::Result<()>;
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
    let task = tokio::spawn(async move {
        let mut old = state.borrow().clone();
        while state.changed().await.is_ok() {
            let new = state.borrow_and_update().clone();
            if let Ok(interface) = bus.object_server().interface::<_, Player>(PATH).await {
                let player = interface.get().await;
                let emitter = interface.signal_emitter();
                if old.playback.playing != new.playback.playing
                    || old.playback.phase != new.playback.phase
                    || old.playback.track != new.playback.track
                {
                    let _ = player.playback_status_changed(emitter).await;
                }
                if old.playback.track != new.playback.track
                    || old.current_entry_id != new.current_entry_id
                {
                    let _ = player.metadata_changed(emitter).await;
                }
                if (old.playback.volume - new.playback.volume).abs() > 0.001 {
                    let _ = player.volume_changed(emitter).await;
                }
                if old.shuffle != new.shuffle {
                    let _ = player.shuffle_changed(emitter).await;
                }
                if old.repeat != new.repeat {
                    let _ = player.loop_status_changed(emitter).await;
                }
                if old.can_next != new.can_next || old.start_available != new.start_available {
                    let _ = player.can_go_next_changed(emitter).await;
                }
                if old.can_previous != new.can_previous
                    || old.start_available != new.start_available
                {
                    let _ = player.can_go_previous_changed(emitter).await;
                }
                if old.audio_pid != new.audio_pid
                    || old.start_available != new.start_available
                    || old.queue_len != new.queue_len
                    || old.playback.phase != new.playback.phase
                {
                    let _ = player.can_play_changed(emitter).await;
                    let _ = player.can_pause_changed(emitter).await;
                    let _ = player.can_control_changed(emitter).await;
                }
                if old.audio_pid != new.audio_pid || old.playback.duration != new.playback.duration
                {
                    let _ = player.can_seek_changed(emitter).await;
                }
                if (new.playback.position - old.playback.position).abs() > 3.0 {
                    let _ =
                        Player::seeked(emitter, (new.playback.position * 1_000_000.0) as i64).await;
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
        assert!(!player.can_control());
        assert!(!player.can_play());
        assert!(!player.can_pause());
        assert!(!player.can_seek());
        assert!(!player.can_go_next());
        assert!(!player.can_go_previous());
    }
}
