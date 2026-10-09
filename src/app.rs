//! Thin egui shell over the core. No business logic here.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use casparcg_osc_companion_timer::binding::{
    ActiveBinding, Binding, BindingProblem, CountMode, TimeFormat,
};
use casparcg_osc_companion_timer::companion_out::UdpCompanionSink;
use casparcg_osc_companion_timer::config::{self, Config, ConnectionSettings};
use casparcg_osc_companion_timer::engine::{lock_engine, BindingStatus, Engine};
use casparcg_osc_companion_timer::layer_state::STALE_AFTER;
use casparcg_osc_companion_timer::worker::Worker;
use eframe::egui::{self, Color32, RichText};

const OK: Color32 = Color32::from_rgb(90, 190, 110);
const WARN: Color32 = Color32::from_rgb(230, 170, 60);
const ERROR: Color32 = Color32::from_rgb(230, 90, 90);

pub struct App {
    engine: Arc<Mutex<Engine>>,
    worker: Option<Worker>,
    /// What is being edited; only takes effect when Apply is pressed.
    draft: ConnectionSettings,
    applied: ConnectionSettings,
    listener_error: Option<String>,
    sink_error: Option<String>,
    notice: Option<String>,
    save_error: Option<String>,
    last_saved: Config,
}

impl App {
    pub fn new() -> Self {
        let (config, notice) = config::load();
        let engine = Arc::new(Mutex::new(Engine::new(config.bindings.clone())));
        let mut app = Self {
            engine,
            worker: None,
            draft: config.connection.clone(),
            applied: config.connection.clone(),
            listener_error: None,
            sink_error: None,
            notice,
            save_error: None,
            last_saved: config,
        };
        app.apply_connection();
        app
    }

    fn apply_connection(&mut self) {
        let settings = self.draft.clone();

        // Dropping the old worker joins its thread, so do it before locking the engine.
        self.worker = None;
        self.listener_error = match Worker::start(settings.input_port, Arc::clone(&self.engine)) {
            Ok(worker) => {
                self.worker = Some(worker);
                None
            }
            Err(error) => Some(format!(
                "Cannot listen on UDP port {}: {error}",
                settings.input_port
            )),
        };

        let sink = UdpCompanionSink::connect(&settings.companion_host, settings.companion_port);
        let mut engine = lock_engine(&self.engine);
        match sink {
            Ok(sink) => {
                engine.set_sink(Some(Box::new(sink)));
                self.sink_error = None;
            }
            Err(error) => {
                engine.set_sink(None);
                self.sink_error = Some(error.to_string());
            }
        }
        drop(engine);

        self.applied = settings;
    }

    fn save_if_changed(&mut self) {
        let current = Config {
            connection: self.applied.clone(),
            bindings: lock_engine(&self.engine).binding_configs(),
        };
        if current == self.last_saved {
            return;
        }
        self.save_error = config::save(&current)
            .err()
            .map(|e| format!("Could not save settings: {e}"));
        self.last_saved = current;
    }

    fn connection_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connection");
        ui.horizontal(|ui| {
            ui.label("CasparCG OSC in (UDP port):");
            ui.add(egui::DragValue::new(&mut self.draft.input_port).clamp_range(1..=65535));
        });
        ui.horizontal(|ui| {
            ui.label("Companion OSC:");
            ui.add(egui::TextEdit::singleline(&mut self.draft.companion_host).desired_width(160.0));
            ui.label(":");
            ui.add(egui::DragValue::new(&mut self.draft.companion_port).clamp_range(1..=65535));
        });
        ui.horizontal(|ui| {
            let changed = self.draft != self.applied;
            if ui
                .add_enabled(changed, egui::Button::new("Apply"))
                .clicked()
            {
                self.apply_connection();
            }
            match &self.listener_error {
                Some(error) => ui.colored_label(ERROR, error),
                None => {
                    ui.colored_label(OK, format!("Listening on port {}", self.applied.input_port))
                }
            };
        });
        if let Some(error) = &self.sink_error {
            ui.colored_label(ERROR, error);
        }
        if let Some(error) = lock_engine(&self.engine).send_error() {
            ui.colored_label(ERROR, error);
        }
    }

    fn bindings_ui(&mut self, ui: &mut egui::Ui) {
        ui.heading("Bindings");
        let now = Instant::now();
        let mut engine = lock_engine(&self.engine);
        let statuses = engine.statuses(now);

        let mut remove = None;
        for (index, active) in engine.bindings_mut().iter_mut().enumerate() {
            if binding_ui(ui, index, active, &statuses[index]) {
                remove = Some(index);
            }
        }
        if let Some(index) = remove {
            engine.remove_binding(index);
        }
        if ui.button("+ Add binding").clicked() {
            let variable = format!("timer{}", engine.binding_count() + 1);
            engine.add_binding(Binding {
                variable,
                ..Binding::default()
            });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The "last seen" status must keep updating even without user input.
        ctx.request_repaint_after(Duration::from_millis(250));

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if let Some(notice) = &self.notice {
                    ui.colored_label(WARN, notice);
                }
                if let Some(error) = &self.save_error {
                    ui.colored_label(ERROR, error);
                }
                self.connection_ui(ui);
                ui.separator();
                self.bindings_ui(ui);
            });
        });
        self.save_if_changed();
    }
}

/// Draws one binding. Returns `true` if it should be removed.
fn binding_ui(
    ui: &mut egui::Ui,
    index: usize,
    active: &mut ActiveBinding,
    status: &BindingStatus,
) -> bool {
    let mut remove = false;
    let binding = &mut active.config;

    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.label("Channel");
            ui.add(egui::DragValue::new(&mut binding.channel).clamp_range(1..=999));
            ui.label("Layer");
            ui.add(egui::DragValue::new(&mut binding.layer).clamp_range(0..=9999));
            ui.checkbox(&mut binding.show_name, "Clip name");
            ui.checkbox(&mut binding.show_time, "Time");

            egui::ComboBox::from_id_source(("mode", index))
                .selected_text(mode_label(binding.mode))
                .show_ui(ui, |ui| {
                    for mode in [CountMode::Up, CountMode::Down] {
                        ui.selectable_value(&mut binding.mode, mode, mode_label(mode));
                    }
                });
            egui::ComboBox::from_id_source(("format", index))
                .selected_text(format_label(binding.format))
                .show_ui(ui, |ui| {
                    for format in [TimeFormat::MmSs, TimeFormat::HhMmSs] {
                        ui.selectable_value(&mut binding.format, format, format_label(format));
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label("Companion custom variable:");
            ui.add(egui::TextEdit::singleline(&mut binding.variable).desired_width(140.0));
            if status.problem.is_none() {
                ui.label(
                    RichText::new(format!("use $(custom:{}) in a button", binding.variable)).weak(),
                );
            }
            if ui.button("Remove").clicked() {
                remove = true;
            }
        });
        if let Some(problem) = status.problem {
            ui.colored_label(ERROR, problem_text(problem));
        }
        ui.horizontal(|ui| {
            let (color, text) = match status.age {
                None => (WARN, "no data yet".to_owned()),
                Some(age) if age <= STALE_AFTER => (OK, "receiving data".to_owned()),
                Some(age) => (WARN, format!("last seen {} s ago", age.as_secs())),
            };
            ui.colored_label(color, text);
            ui.label(RichText::new(format!("Sending: \"{}\"", status.preview)).monospace());
        });
    });
    remove
}

fn problem_text(problem: BindingProblem) -> &'static str {
    match problem {
        BindingProblem::EmptyVariable => "Enter a variable name. Nothing is sent until you do.",
        BindingProblem::InvalidVariable => {
            "Use only ASCII letters, digits, _ and -. Nothing is sent until the name is valid."
        }
        BindingProblem::DuplicateVariable => {
            "Another binding uses the same variable. Give each binding its own."
        }
        BindingProblem::NothingSelected => {
            "Enable clip name or time. Nothing is sent until you do."
        }
    }
}

fn mode_label(mode: CountMode) -> &'static str {
    match mode {
        CountMode::Up => "Count up",
        CountMode::Down => "Count down",
    }
}

fn format_label(format: TimeFormat) -> &'static str {
    match format {
        TimeFormat::MmSs => "mm:ss",
        TimeFormat::HhMmSs => "hh:mm:ss",
    }
}
