//! Service-owned session. Rows are in the exact, stable playback order.
use crate::model::{Repeat, Track};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAX_QUEUE: usize = 10_000;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueContext {
    pub provider: String,
    pub storefront: String,
    pub album_id: Option<String>,
    pub playlist_id: Option<String>,
    pub title: String,
}
impl QueueContext {
    pub fn single() -> Self {
        Self {
            title: "Single song".into(),
            ..Self::default()
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueEntry {
    pub entry_id: u64,
    #[serde(flatten)]
    pub track: Track,
    pub context: QueueContext,
}
impl std::ops::Deref for QueueEntry {
    type Target = Track;
    fn deref(&self) -> &Track {
        &self.track
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Queue {
    pub tracks: Vec<QueueEntry>,
    pub current: Option<usize>,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub revision: u64,
    pub next_entry_id: u64,
    history: Vec<u64>,
    forward: Vec<u64>,
    priority: Vec<u64>,
    stop_after_current: bool,
}
impl Queue {
    pub fn changed(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }
    pub fn current_entry_id(&self) -> Option<u64> {
        self.current
            .and_then(|i| self.tracks.get(i))
            .map(|e| e.entry_id)
    }
    pub fn current_track(&self) -> Option<&Track> {
        self.current
            .and_then(|i| self.tracks.get(i))
            .map(|e| &e.track)
    }
    pub fn index(&self, id: u64) -> Option<usize> {
        self.tracks.iter().position(|e| e.entry_id == id)
    }
    pub fn migrate(&mut self) -> Result<()> {
        ensure!(self.tracks.len() <= MAX_QUEUE, "queue limit reached");
        let mut seen = std::collections::HashSet::new();
        let legacy = self.tracks.iter().all(|e| e.entry_id == 0);
        if legacy {
            for (i, e) in self.tracks.iter_mut().enumerate() {
                e.entry_id = i as u64 + 1;
            }
            self.next_entry_id = self.tracks.len() as u64;
        }
        for e in &self.tracks {
            ensure!(
                e.entry_id > 0 && seen.insert(e.entry_id),
                "invalid queue entry identity"
            );
        }
        self.next_entry_id = self
            .next_entry_id
            .max(self.tracks.iter().map(|e| e.entry_id).max().unwrap_or(0));
        self.current = self.current.filter(|i| *i < self.tracks.len());
        ensure!(
            self.history.len() <= MAX_QUEUE
                && self.forward.len() <= MAX_QUEUE
                && self.priority.len() <= MAX_QUEUE,
            "queue history limit reached"
        );
        ensure!(
            self.history
                .iter()
                .chain(&self.forward)
                .chain(&self.priority)
                .all(|id| seen.contains(id)),
            "invalid queue history"
        );
        Ok(())
    }
    fn entries(&mut self, tracks: Vec<Track>, context: QueueContext) -> Result<Vec<QueueEntry>> {
        ensure!(
            self.tracks.len().saturating_add(tracks.len()) <= MAX_QUEUE,
            "queue is full (10000 tracks)"
        );
        ensure!(
            self.next_entry_id
                .checked_add(tracks.len() as u64)
                .is_some(),
            "queue identity exhausted"
        );
        Ok(tracks
            .into_iter()
            .map(|track| {
                self.next_entry_id += 1;
                QueueEntry {
                    entry_id: self.next_entry_id,
                    track,
                    context: context.clone(),
                }
            })
            .collect())
    }
    pub fn push(&mut self, track: Track) -> Result<()> {
        self.append(vec![track], QueueContext::single())
    }
    pub fn append(&mut self, tracks: Vec<Track>, context: QueueContext) -> Result<()> {
        let entries = self.entries(tracks, context)?;
        self.tracks.extend(entries);
        self.stop_after_current = false;
        self.changed();
        Ok(())
    }
    pub fn insert_next(&mut self, tracks: Vec<Track>, context: QueueContext) -> Result<()> {
        let entries = self.entries(tracks, context)?;
        let start = self.current.map_or(0, |i| i + 1);
        let count = self
            .tracks
            .iter()
            .skip(start)
            .take_while(|e| self.priority.contains(&e.entry_id))
            .count();
        self.priority.extend(entries.iter().map(|e| e.entry_id));
        self.tracks.splice(start + count..start + count, entries);
        self.stop_after_current = false;
        self.changed();
        Ok(())
    }
    pub fn replace(
        &mut self,
        tracks: Vec<Track>,
        start: usize,
        shuffle: bool,
        context: QueueContext,
    ) -> Result<()> {
        ensure!(
            !tracks.is_empty() && start < tracks.len() && tracks.len() <= MAX_QUEUE,
            "invalid playback context"
        );
        let mut next = Self {
            repeat: self.repeat,
            revision: self.revision,
            next_entry_id: self.next_entry_id,
            ..Self::default()
        };
        next.append(tracks, context)?;
        next.current = Some(start);
        if shuffle {
            // A random album starts anywhere; selecting a particular song keeps that song first.
            next.tracks.swap(0, start);
            next.current = Some(0);
            next.set_shuffle(true);
        }
        next.revision = self.revision.saturating_add(1);
        *self = next;
        Ok(())
    }
    pub fn select(&mut self, index: usize) -> bool {
        if index >= self.tracks.len() {
            return false;
        }
        if let Some(old) = self.current_entry_id()
            && self.current != Some(index)
        {
            self.history.push(old);
            if self.history.len() > MAX_QUEUE {
                self.history.remove(0);
            }
        }
        self.current = Some(index);
        self.forward.clear();
        let id = self.tracks[index].entry_id;
        self.priority.retain(|v| *v != id);
        self.changed();
        true
    }
    pub fn set_shuffle(&mut self, enabled: bool) {
        if self.shuffle == enabled {
            return;
        }
        let start = self.current.map_or(0, |i| i + 1);
        let count = self
            .tracks
            .iter()
            .skip(start)
            .take_while(|e| self.priority.contains(&e.entry_id))
            .count();
        let tail = &mut self.tracks[start + count..];
        if enabled {
            fastrand::shuffle(tail);
        } else {
            tail.sort_by_key(|e| e.entry_id);
        }
        self.shuffle = enabled;
        self.changed();
    }
    pub fn remove(&mut self, i: usize) -> Option<QueueEntry> {
        if i >= self.tracks.len() {
            return None;
        }
        self.current = match self.current {
            Some(c) if c == i => None,
            Some(c) if c > i => Some(c - 1),
            c => c,
        };
        let entry = self.tracks.remove(i);
        self.history.retain(|v| *v != entry.entry_id);
        self.forward.retain(|v| *v != entry.entry_id);
        self.priority.retain(|v| *v != entry.entry_id);
        self.changed();
        Some(entry)
    }
    pub fn move_item(&mut self, from: usize, to: usize) -> bool {
        if from >= self.tracks.len() || to >= self.tracks.len() {
            return false;
        }
        let current = self.current_entry_id();
        let entry = self.tracks.remove(from);
        self.tracks.insert(to, entry);
        self.current = current.and_then(|id| self.index(id));
        self.forward.clear();
        self.changed();
        true
    }
    pub fn clear_upcoming(&mut self) {
        let current = self.current.and_then(|i| self.tracks.get(i)).cloned();
        self.tracks.clear();
        if let Some(entry) = current {
            self.tracks.push(entry);
            self.current = Some(0);
        } else {
            self.current = None;
        }
        self.history.clear();
        self.forward.clear();
        self.priority.clear();
        self.stop_after_current = true;
        self.changed();
    }
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.current = None;
        self.history.clear();
        self.forward.clear();
        self.priority.clear();
        self.stop_after_current = false;
        self.changed();
    }
    pub fn candidate(&self, manual: bool) -> Option<usize> {
        if self.tracks.is_empty() || self.stop_after_current {
            return None;
        }
        if !manual && self.repeat == Repeat::One {
            return self.current.or(Some(0));
        }
        if !self.forward.is_empty()
            && let Some(next) = self
                .current
                .and_then(|i| i.checked_add(1))
                .filter(|i| *i < self.tracks.len())
            && self.priority.contains(&self.tracks[next].entry_id)
        {
            return Some(next);
        }
        if let Some(id) = self.forward.last() {
            return self.index(*id);
        }
        match self.current {
            None => Some(0),
            Some(i) if i + 1 < self.tracks.len() => Some(i + 1),
            Some(_) if self.repeat == Repeat::All => Some(0),
            _ => None,
        }
    }
    /// One visible manual traversal, stopping before the next repeat-all cycle.
    pub fn upcoming_ids(&self) -> Vec<u64> {
        if self.stop_after_current || self.tracks.is_empty() {
            return Vec::new();
        }
        let mut result = Vec::new();
        let mut current = self.current;
        let mut forward = self.forward.clone();
        let mut priority = self
            .priority
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        let mut seen = std::collections::HashSet::new();
        if let Some(id) = self.current_entry_id() {
            seen.insert(id);
        }
        loop {
            let adjacent = current.map_or(Some(0), |i| {
                if i + 1 < self.tracks.len() {
                    Some(i + 1)
                } else if self.repeat == Repeat::All {
                    Some(0)
                } else {
                    None
                }
            });
            let next = if !forward.is_empty()
                && adjacent.is_some_and(|i| priority.contains(&self.tracks[i].entry_id))
            {
                adjacent
            } else if let Some(id) = forward.last() {
                self.index(*id)
            } else {
                adjacent
            };
            let Some(index) = next else {
                break;
            };
            let id = self.tracks[index].entry_id;
            if !seen.insert(id) {
                break;
            }
            result.push(id);
            priority.remove(&id);
            if forward.last() == Some(&id) {
                forward.pop();
            }
            current = Some(index);
        }
        result
    }
    pub fn advance(&mut self, manual: bool) -> Option<usize> {
        let next = self.candidate(manual)?;
        let forward = self.forward.clone();
        self.select(next);
        if !forward.is_empty() {
            self.forward = forward;
            if self.forward.last() == self.current_entry_id().as_ref() {
                self.forward.pop();
            }
        }
        Some(next)
    }
    pub fn previous_candidate(&self) -> Option<usize> {
        if self.shuffle {
            self.history.last().and_then(|id| self.index(*id))
        } else {
            self.current.and_then(|i| i.checked_sub(1))
        }
    }
    pub fn previous(&mut self) -> Option<usize> {
        let previous = self.previous_candidate()?;
        if let Some(id) = self.current_entry_id() {
            self.forward.push(id);
        }
        self.history.pop();
        self.current = Some(previous);
        self.changed();
        Some(previous)
    }
    pub fn can_resume(&self, track: Option<&Track>) -> bool {
        self.current_track()
            .zip(track)
            .is_some_and(|(a, b)| a.id == b.id)
    }
}
