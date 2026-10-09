//! Ties state, bindings and the Companion gateway together.
//! Driven by the worker thread; the GUI only locks it briefly.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use crate::binding::{ActiveBinding, Binding, BindingProblem};
use crate::caspar_in::LayerUpdate;
use crate::companion_out::CompanionSink;
use crate::layer_state::{LayerKey, LayerStates};

pub type BoxedSink = Box<dyn CompanionSink + Send>;

/// What the GUI shows per binding.
pub struct BindingStatus {
    /// `None` means no data has ever been seen for the layer.
    pub age: Option<Duration>,
    /// The text the binding is sending right now.
    pub preview: String,
    /// If set, the binding is not sending anything.
    pub problem: Option<BindingProblem>,
}

pub struct Engine {
    states: LayerStates,
    bindings: Vec<ActiveBinding>,
    sink: Option<BoxedSink>,
    send_error: Option<String>,
}

impl Engine {
    pub fn new(bindings: Vec<Binding>) -> Self {
        Self {
            states: LayerStates::default(),
            bindings: bindings.into_iter().map(ActiveBinding::new).collect(),
            sink: None,
            send_error: None,
        }
    }

    /// Early filter: is there a binding on this layer?
    pub fn watches(&self, key: LayerKey) -> bool {
        self.bindings.iter().any(|b| b.config.key() == key)
    }

    pub fn apply(&mut self, updates: Vec<LayerUpdate>, now: Instant) {
        for (key, update) in updates {
            self.states.apply(key, update, now);
        }
    }

    /// Sends every binding whose text has changed (including the timeout to empty text).
    pub fn refresh(&mut self, now: Instant) {
        let problems = self.problems();
        let bindings = &self.bindings;
        self.states
            .retain(|key| bindings.iter().any(|b| b.config.key() == key));

        for (active, problem) in self.bindings.iter_mut().zip(problems) {
            if problem.is_some() {
                continue;
            }
            let state = self.states.get(active.config.key());
            let Some(text) = active.text_to_send(state, now) else {
                continue;
            };
            let Some(sink) = self.sink.as_mut() else {
                continue;
            };
            match sink.set_variable(&active.config.variable, &text) {
                Ok(()) => {
                    active.mark_sent(text);
                    self.send_error = None;
                }
                Err(error) => self.send_error = Some(error.to_string()),
            }
        }
    }

    /// New target: everything must be sent again, even if the text is unchanged.
    pub fn set_sink(&mut self, sink: Option<BoxedSink>) {
        self.sink = sink;
        self.send_error = None;
        for active in &mut self.bindings {
            active.forget_sent();
        }
    }

    pub fn send_error(&self) -> Option<&str> {
        self.send_error.as_deref()
    }

    pub fn bindings_mut(&mut self) -> &mut [ActiveBinding] {
        &mut self.bindings
    }

    pub fn binding_count(&self) -> usize {
        self.bindings.len()
    }

    pub fn add_binding(&mut self, binding: Binding) {
        self.bindings.push(ActiveBinding::new(binding));
    }

    pub fn remove_binding(&mut self, index: usize) {
        if index < self.bindings.len() {
            self.bindings.remove(index);
        }
    }

    pub fn binding_configs(&self) -> Vec<Binding> {
        self.bindings.iter().map(|b| b.config.clone()).collect()
    }

    pub fn statuses(&self, now: Instant) -> Vec<BindingStatus> {
        let problems = self.problems();
        self.bindings
            .iter()
            .zip(problems)
            .map(|(active, problem)| {
                let state = self.states.get(active.config.key());
                BindingStatus {
                    age: state.map(|s| s.age(now)),
                    preview: active.config.render(state, now),
                    problem,
                }
            })
            .collect()
    }

    /// Per binding: why it must not send. Two bindings on one variable would
    /// overwrite each other every second, so both are blocked.
    fn problems(&self) -> Vec<Option<BindingProblem>> {
        self.bindings
            .iter()
            .map(|active| {
                active.config.problem().or_else(|| {
                    let name = &active.config.variable;
                    let sharing = self
                        .bindings
                        .iter()
                        .filter(|other| other.config.variable == *name)
                        .count();
                    (sharing > 1).then_some(BindingProblem::DuplicateVariable)
                })
            })
            .collect()
    }
}

/// Locking that does not panic if another thread went down while holding the lock.
pub fn lock_engine(engine: &Arc<Mutex<Engine>>) -> MutexGuard<'_, Engine> {
    engine.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::CountMode;
    use crate::companion_out::SinkError;
    use crate::layer_state::ClipUpdate;

    type Sent = Arc<Mutex<Vec<(String, String)>>>;

    struct RecordingSink(Sent);

    impl CompanionSink for RecordingSink {
        fn set_variable(&mut self, name: &str, text: &str) -> Result<(), SinkError> {
            self.0
                .lock()
                .unwrap()
                .push((name.to_owned(), text.to_owned()));
            Ok(())
        }
    }

    const KEY: LayerKey = LayerKey {
        channel: 1,
        layer: 10,
    };

    fn bound(variable: &str, mode: CountMode) -> Binding {
        Binding {
            mode,
            variable: variable.into(),
            ..Binding::default()
        }
    }

    fn engine_with_sink(bindings: Vec<Binding>) -> (Engine, Sent) {
        let sent: Sent = Arc::default();
        let mut engine = Engine::new(bindings);
        engine.set_sink(Some(Box::new(RecordingSink(Arc::clone(&sent)))));
        (engine, sent)
    }

    fn time(elapsed: f64, total: f64) -> Vec<LayerUpdate> {
        vec![(KEY, ClipUpdate::Time { elapsed, total })]
    }

    fn name(value: &str) -> Vec<LayerUpdate> {
        vec![(KEY, ClipUpdate::Name(Some(value.into())))]
    }

    fn texts(sent: &Sent) -> Vec<String> {
        sent.lock()
            .unwrap()
            .iter()
            .map(|(_, t)| t.clone())
            .collect()
    }

    fn start_clip(engine: &mut Engine, now: Instant, elapsed: f64) {
        engine.apply(name("AMB"), now);
        engine.apply(time(elapsed, 10.0), now);
        engine.refresh(now);
    }

    #[test]
    fn watches_only_bound_layers() {
        let engine = Engine::new(vec![Binding::default()]);
        assert!(engine.watches(KEY));
        assert!(!engine.watches(LayerKey {
            channel: 2,
            layer: 10
        }));
    }

    #[test]
    fn sends_once_per_displayed_second() {
        let (mut engine, sent) = engine_with_sink(vec![bound("timer1", CountMode::Up)]);
        let now = Instant::now();

        engine.apply(name("AMB"), now);
        for elapsed in [1.0, 1.04, 1.08, 1.92, 2.0, 2.04] {
            engine.apply(time(elapsed, 10.0), now);
            engine.refresh(now);
        }

        // Six time updates, but only two distinct displayed seconds.
        assert_eq!(texts(&sent), vec!["AMB - 00:01", "AMB - 00:02"]);
    }

    #[test]
    fn up_and_down_bindings_write_to_their_own_variables() {
        let (mut engine, sent) = engine_with_sink(vec![
            bound("up1", CountMode::Up),
            bound("down1", CountMode::Down),
        ]);
        start_clip(&mut engine, Instant::now(), 4.0);

        assert_eq!(
            *sent.lock().unwrap(),
            vec![
                ("up1".to_owned(), "AMB - 00:04".to_owned()),
                ("down1".to_owned(), "AMB - 00:06".to_owned()),
            ]
        );
    }

    #[test]
    fn clears_variable_when_layer_goes_quiet() {
        let (mut engine, sent) = engine_with_sink(vec![bound("timer1", CountMode::Down)]);
        let now = Instant::now();

        start_clip(&mut engine, now, 1.0);
        engine.refresh(now + Duration::from_secs(5));
        engine.refresh(now + Duration::from_secs(6));

        assert_eq!(texts(&sent), vec!["AMB - 00:09", ""]);
    }

    #[test]
    fn new_sink_gets_everything_again() {
        let (mut engine, sent) = engine_with_sink(vec![bound("timer1", CountMode::Down)]);
        let now = Instant::now();
        start_clip(&mut engine, now, 1.0);

        let second: Sent = Arc::default();
        engine.set_sink(Some(Box::new(RecordingSink(Arc::clone(&second)))));
        engine.refresh(now);

        assert_eq!(texts(&sent).len(), 1);
        assert_eq!(texts(&second), vec!["AMB - 00:09"]);
    }

    #[test]
    fn renaming_the_variable_resends_the_current_text() {
        let (mut engine, sent) = engine_with_sink(vec![bound("timer1", CountMode::Down)]);
        let now = Instant::now();
        start_clip(&mut engine, now, 1.0);

        engine.bindings_mut()[0].config.variable = "timer2".into();
        engine.refresh(now);

        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1], ("timer2".to_owned(), "AMB - 00:09".to_owned()));
    }

    #[test]
    fn bindings_without_a_valid_variable_send_nothing() {
        for variable in ["", "a b", "a/b"] {
            let (mut engine, sent) = engine_with_sink(vec![bound(variable, CountMode::Up)]);
            start_clip(&mut engine, Instant::now(), 1.0);
            assert!(sent.lock().unwrap().is_empty(), "{variable:?}");
        }
    }

    #[test]
    fn bindings_sharing_a_variable_are_flagged_and_send_nothing() {
        let (mut engine, sent) = engine_with_sink(vec![
            bound("timer1", CountMode::Up),
            bound("timer1", CountMode::Down),
            bound("timer2", CountMode::Up),
        ]);
        let now = Instant::now();
        start_clip(&mut engine, now, 1.0);

        let problems: Vec<_> = engine
            .statuses(now)
            .into_iter()
            .map(|s| s.problem)
            .collect();
        assert_eq!(
            problems,
            vec![
                Some(BindingProblem::DuplicateVariable),
                Some(BindingProblem::DuplicateVariable),
                None
            ]
        );
        let sent = sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "timer2");
    }

    #[test]
    fn status_reports_no_data_until_layer_is_seen() {
        let mut engine = Engine::new(vec![Binding::default()]);
        let now = Instant::now();
        assert!(engine.statuses(now)[0].age.is_none());

        engine.apply(name("AMB"), now);
        assert!(engine.statuses(now)[0].age.is_some());
    }

    #[test]
    fn removed_binding_forgets_its_state() {
        let mut engine = Engine::new(vec![Binding::default()]);
        let now = Instant::now();
        engine.apply(name("AMB"), now);
        engine.remove_binding(0);
        engine.refresh(now);
        engine.add_binding(Binding::default());
        assert!(engine.statuses(now)[0].age.is_none());
    }
}
