//! A binding connects one (channel, layer) to one Companion custom variable.
//! Rendering is pure: the same state always gives the same text.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::layer_state::{ClipState, LayerKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountMode {
    /// Elapsed time (rounded down).
    Up,
    /// Time remaining (rounded up, so it reads 00:01 until the last second is used).
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimeFormat {
    MmSs,
    HhMmSs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Binding {
    pub channel: u32,
    pub layer: u32,
    pub mode: CountMode,
    pub format: TimeFormat,
    /// Name of the Companion custom variable that receives the text.
    pub variable: String,
    pub show_name: bool,
    pub show_time: bool,
}

impl Default for Binding {
    fn default() -> Self {
        Self {
            channel: 1,
            layer: 10,
            mode: CountMode::Down,
            format: TimeFormat::MmSs,
            variable: String::new(),
            show_name: true,
            show_time: true,
        }
    }
}

/// Why a binding is not sending anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingProblem {
    EmptyVariable,
    InvalidVariable,
    NothingSelected,
    DuplicateVariable,
}

impl Binding {
    pub fn key(&self) -> LayerKey {
        LayerKey {
            channel: self.channel,
            layer: self.layer,
        }
    }

    pub fn problem(&self) -> Option<BindingProblem> {
        if self.variable.is_empty() {
            Some(BindingProblem::EmptyVariable)
        } else if !is_valid_variable_name(&self.variable) {
            Some(BindingProblem::InvalidVariable)
        } else if !self.show_name && !self.show_time {
            Some(BindingProblem::NothingSelected)
        } else {
            None
        }
    }

    /// Variable text for the given state. Empty text means "nothing is playing".
    pub fn render(&self, state: Option<&ClipState>, now: Instant) -> String {
        let Some(state) = state.filter(|state| state.is_live(now)) else {
            return String::new();
        };
        let Some(name) = state.name() else {
            return String::new();
        };
        let mut parts = Vec::with_capacity(2);
        if self.show_name {
            parts.push(name.to_owned());
        }
        if self.show_time {
            parts.push(self.time_text(state));
        }
        parts.join(" ")
    }

    fn time_text(&self, state: &ClipState) -> String {
        match self.mode {
            CountMode::Up => format_time(state.elapsed().max(0.0).floor() as u64, self.format),
            // Without a known length (live input, HTML template) we cannot count down.
            CountMode::Down if state.total() <= 0.0 => placeholder(self.format),
            CountMode::Down => {
                let remaining = (state.total() - state.elapsed()).max(0.0).ceil();
                format_time(remaining as u64, self.format)
            }
        }
    }
}

/// Conservative: ASCII letters, digits, `_` and `-`. This keeps the OSC
/// address well-formed; it is not Companion's official naming rule.
pub fn is_valid_variable_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub fn format_time(seconds: u64, format: TimeFormat) -> String {
    match format {
        TimeFormat::MmSs => format!("{:02}:{:02}", seconds / 60, seconds % 60),
        TimeFormat::HhMmSs => format!(
            "{:02}:{:02}:{:02}",
            seconds / 3600,
            (seconds / 60) % 60,
            seconds % 60
        ),
    }
}

fn placeholder(format: TimeFormat) -> String {
    match format {
        TimeFormat::MmSs => "--:--".into(),
        TimeFormat::HhMmSs => "--:--:--".into(),
    }
}

/// A binding plus what we last sent for it (used for dedupe).
pub struct ActiveBinding {
    pub config: Binding,
    last_sent: Option<(String, String)>,
}

impl ActiveBinding {
    pub fn new(config: Binding) -> Self {
        Self {
            config,
            last_sent: None,
        }
    }

    /// The text to send now, or `None` if Companion already has it.
    pub fn text_to_send(&self, state: Option<&ClipState>, now: Instant) -> Option<String> {
        let text = self.config.render(state, now);
        let unchanged = self
            .last_sent
            .as_ref()
            .is_some_and(|(variable, sent)| *variable == self.config.variable && *sent == text);
        (!unchanged).then_some(text)
    }

    pub fn mark_sent(&mut self, text: String) {
        self.last_sent = Some((self.config.variable.clone(), text));
    }

    pub fn forget_sent(&mut self) {
        self.last_sent = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layer_state::{ClipUpdate, LayerStates};
    use std::time::Duration;

    const KEY: LayerKey = LayerKey {
        channel: 1,
        layer: 10,
    };
    #[test]
    fn name_and_time_can_be_switched_off_independently() {
        let now = Instant::now();
        let states = states_with(Some("AMB"), 4.0, 10.0, now);
        let mut binding = binding(CountMode::Up, TimeFormat::MmSs);

        binding.show_time = false;
        assert_eq!(binding.render(states.get(KEY), now), "AMB");

        binding.show_name = false;
        binding.show_time = true;
        assert_eq!(binding.render(states.get(KEY), now), "00:04");

        // An empty layer stays empty, whatever is switched on.
        let empty = states_with(None, 0.0, 0.0, now);
        assert_eq!(binding.render(empty.get(KEY), now), "");
    }

    #[test]
    fn switching_off_both_is_a_problem() {
        let mut binding = binding(CountMode::Up, TimeFormat::MmSs);
        binding.show_name = false;
        binding.show_time = false;
        assert_eq!(binding.problem(), Some(BindingProblem::NothingSelected));
    }

    fn states_with(name: Option<&str>, elapsed: f64, total: f64, now: Instant) -> LayerStates {
        let mut states = LayerStates::default();
        states.apply(KEY, ClipUpdate::Name(name.map(Into::into)), now);
        states.apply(KEY, ClipUpdate::Time { elapsed, total }, now);
        states
    }

    fn binding(mode: CountMode, format: TimeFormat) -> Binding {
        Binding {
            mode,
            format,
            variable: "timer1".into(),
            ..Binding::default()
        }
    }

    #[test]
    fn formats_time() {
        assert_eq!(format_time(0, TimeFormat::MmSs), "00:00");
        assert_eq!(format_time(75, TimeFormat::MmSs), "01:15");
        assert_eq!(format_time(3600 + 125, TimeFormat::MmSs), "62:05");
        assert_eq!(format_time(3600 + 125, TimeFormat::HhMmSs), "01:02:05");
    }

    #[test]
    fn counts_up_rounding_down() {
        let now = Instant::now();
        let states = states_with(Some("AMB"), 3.9, 10.0, now);
        let up = binding(CountMode::Up, TimeFormat::MmSs);
        assert_eq!(up.render(states.get(KEY), now), "AMB - 00:03");
    }

    #[test]
    fn counts_down_rounding_up() {
        let now = Instant::now();
        let down = binding(CountMode::Down, TimeFormat::MmSs);

        let states = states_with(Some("AMB"), 3.5, 10.0, now);
        assert_eq!(down.render(states.get(KEY), now), "AMB - 00:07");

        let states = states_with(Some("AMB"), 9.6, 10.0, now);
        assert_eq!(down.render(states.get(KEY), now), "AMB - 00:01");

        let states = states_with(Some("AMB"), 10.0, 10.0, now);
        assert_eq!(down.render(states.get(KEY), now), "AMB - 00:00");
    }

    #[test]
    fn countdown_without_known_length_shows_placeholder() {
        let now = Instant::now();
        let states = states_with(Some("LIVE"), 5.0, 0.0, now);
        let down = binding(CountMode::Down, TimeFormat::HhMmSs);
        assert_eq!(down.render(states.get(KEY), now), "LIVE - --:--:--");
    }

    #[test]
    fn idle_layer_renders_empty() {
        let now = Instant::now();
        let up = binding(CountMode::Up, TimeFormat::MmSs);

        assert_eq!(up.render(None, now), "");

        let empty = states_with(None, 0.0, 0.0, now);
        assert_eq!(up.render(empty.get(KEY), now), "");

        let stale = states_with(Some("AMB"), 3.0, 10.0, now);
        assert_eq!(up.render(stale.get(KEY), now + Duration::from_secs(5)), "");
    }

    #[test]
    fn up_and_down_can_coexist_on_one_layer() {
        let now = Instant::now();
        let states = states_with(Some("AMB"), 4.0, 10.0, now);
        let up = binding(CountMode::Up, TimeFormat::MmSs);
        let down = binding(CountMode::Down, TimeFormat::MmSs);
        assert_eq!(up.render(states.get(KEY), now), "AMB - 00:04");
        assert_eq!(down.render(states.get(KEY), now), "AMB - 00:06");
    }

    #[test]
    fn validates_variable_names() {
        for valid in ["timer1", "Clip_A", "a-b", "X"] {
            assert!(is_valid_variable_name(valid), "{valid}");
        }
        for invalid in ["", "a b", "a/b", "tid:1", "æble", "$(custom:x)"] {
            assert!(!is_valid_variable_name(invalid), "{invalid}");
        }
    }

    #[test]
    fn reports_variable_problems() {
        let mut binding = Binding::default();
        assert_eq!(binding.problem(), Some(BindingProblem::EmptyVariable));
        binding.variable = "a b".into();
        assert_eq!(binding.problem(), Some(BindingProblem::InvalidVariable));
        binding.variable = "timer1".into();
        assert_eq!(binding.problem(), None);
    }

    #[test]
    fn only_sends_when_text_or_variable_changes() {
        let now = Instant::now();
        let mut active = ActiveBinding::new(binding(CountMode::Up, TimeFormat::MmSs));

        let states = states_with(Some("AMB"), 3.1, 10.0, now);
        let text = active
            .text_to_send(states.get(KEY), now)
            .expect("first send");
        active.mark_sent(text);

        let states = states_with(Some("AMB"), 3.8, 10.0, now);
        assert!(active.text_to_send(states.get(KEY), now).is_none());

        let states = states_with(Some("AMB"), 4.0, 10.0, now);
        assert!(active.text_to_send(states.get(KEY), now).is_some());

        active.config.variable = "timer2".into();
        assert!(active.text_to_send(states.get(KEY), now).is_some());
    }
}
