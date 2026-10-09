//! What we know about the clip on a given (channel, layer).

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long without messages before a layer counts as empty.
pub const STALE_AFTER: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerKey {
    pub channel: u32,
    pub layer: u32,
}

/// A change in clip state, independent of how it was transported.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipUpdate {
    /// `None` means the layer is empty.
    Name(Option<String>),
    /// Seconds.
    Time { elapsed: f64, total: f64 },
}

#[derive(Clone, Debug)]
pub struct ClipState {
    name: Option<String>,
    elapsed: f64,
    total: f64,
    last_seen: Instant,
}

impl ClipState {
    fn new(now: Instant) -> Self {
        Self { name: None, elapsed: 0.0, total: 0.0, last_seen: now }
    }

    fn apply(&mut self, update: ClipUpdate, now: Instant) {
        match update {
            ClipUpdate::Name(name) => self.name = name,
            ClipUpdate::Time { elapsed, total } => {
                self.elapsed = elapsed;
                self.total = total;
            }
        }
        self.last_seen = now;
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    pub fn total(&self) -> f64 {
        self.total
    }

    pub fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_seen)
    }

    pub fn is_live(&self, now: Instant) -> bool {
        self.age(now) <= STALE_AFTER
    }
}

#[derive(Default)]
pub struct LayerStates {
    states: HashMap<LayerKey, ClipState>,
}

impl LayerStates {
    pub fn apply(&mut self, key: LayerKey, update: ClipUpdate, now: Instant) {
        self.states
            .entry(key)
            .or_insert_with(|| ClipState::new(now))
            .apply(update, now);
    }

    pub fn get(&self, key: LayerKey) -> Option<&ClipState> {
        self.states.get(&key)
    }

    /// Forgets layers that are no longer watched.
    pub fn retain(&mut self, keep: impl Fn(LayerKey) -> bool) {
        self.states.retain(|key, _| keep(*key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: LayerKey = LayerKey { channel: 1, layer: 10 };

    #[test]
    fn unknown_layer_has_no_state() {
        assert!(LayerStates::default().get(KEY).is_none());
    }

    #[test]
    fn updates_accumulate_per_layer() {
        let now = Instant::now();
        let mut states = LayerStates::default();
        states.apply(KEY, ClipUpdate::Name(Some("AMB".into())), now);
        states.apply(KEY, ClipUpdate::Time { elapsed: 3.0, total: 10.0 }, now);

        let state = states.get(KEY).unwrap();
        assert_eq!(state.name(), Some("AMB"));
        assert_eq!(state.elapsed(), 3.0);
        assert_eq!(state.total(), 10.0);
    }

    #[test]
    fn layer_goes_stale_without_messages() {
        let now = Instant::now();
        let mut states = LayerStates::default();
        states.apply(KEY, ClipUpdate::Name(Some("AMB".into())), now);

        let state = states.get(KEY).unwrap();
        assert!(state.is_live(now + Duration::from_secs(1)));
        assert!(!state.is_live(now + Duration::from_secs(3)));
    }

    #[test]
    fn retain_forgets_unwatched_layers() {
        let now = Instant::now();
        let mut states = LayerStates::default();
        states.apply(KEY, ClipUpdate::Name(None), now);
        states.retain(|_| false);
        assert!(states.get(KEY).is_none());
    }
}
