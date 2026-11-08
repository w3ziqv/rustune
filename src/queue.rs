use crate::model::{Repeat, Track};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Queue {
    pub tracks: Vec<Track>,
    pub current: Option<usize>,
    pub repeat: Repeat,
    pub shuffle: bool,
    #[serde(default)]
    pub revision: u64,
}
impl Queue {
    pub fn push(&mut self, t: Track) -> anyhow::Result<()> {
        anyhow::ensure!(self.tracks.len() < 10_000, "queue is full (10000 tracks)");
        self.tracks.push(t);
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }
    pub fn remove(&mut self, i: usize) -> Option<Track> {
        if i >= self.tracks.len() {
            return None;
        }
        self.current = match self.current {
            Some(c) if c == i => None,
            Some(c) if c > i => Some(c - 1),
            c => c,
        };
        let track = self.tracks.remove(i);
        self.revision = self.revision.wrapping_add(1);
        Some(track)
    }
    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tracks.len() || to >= self.tracks.len() {
            return false;
        }
        if from == to {
            return true;
        }
        let t = self.tracks.remove(from);
        self.tracks.insert(to, t);
        self.revision = self.revision.wrapping_add(1);
        self.current = self.current.map(|i| {
            if i == from {
                to
            } else if from < i && i <= to {
                i - 1
            } else if to <= i && i < from {
                i + 1
            } else {
                i
            }
        });
        true
    }
    pub fn clear(&mut self) {
        *self = Self {
            revision: self.revision.wrapping_add(1),
            ..Self::default()
        };
    }
    pub fn candidate(&self, manual: bool) -> Option<usize> {
        let len = self.tracks.len();
        if len == 0 {
            return None;
        }
        let c = self.current.filter(|i| *i < len);
        if self.repeat == Repeat::One && !manual {
            c.or(Some(0))
        } else if self.shuffle && len > 1 {
            let mut i = fastrand::usize(..len - 1);
            if c.is_some_and(|c| i >= c) {
                i += 1;
            }
            Some(i)
        } else {
            match c {
                None => Some(0),
                Some(i) if i + 1 < len => Some(i + 1),
                Some(_) if self.repeat == Repeat::All => Some(0),
                _ => None,
            }
        }
    }
    pub fn advance(&mut self, manual: bool) -> Option<usize> {
        let next = self.candidate(manual);
        if next.is_some() {
            self.current = next;
        }
        next
    }
    pub fn can_resume(&self, track: Option<&Track>) -> bool {
        self.current
            .and_then(|i| self.tracks.get(i))
            .zip(track)
            .is_some_and(|(a, b)| a.id == b.id)
    }
}
