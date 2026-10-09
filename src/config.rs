//! Configuration as JSON in the user's config directory.

use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::binding::Binding;

const APP_DIR: &str = "casparcg-osc-companion-timer";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConnectionSettings {
    /// UDP port that CasparCG's OSC client points at.
    pub input_port: u16,
    pub companion_host: String,
    /// Companion's OSC port.
    pub companion_port: u16,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            input_port: 6250,
            companion_host: "127.0.0.1".into(),
            companion_port: 12321,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub connection: ConnectionSettings,
    pub bindings: Vec<Binding>,
}

pub fn config_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join(APP_DIR).join("config.json"))
}

/// Reads the config. An unreadable file is moved to `.bak` so the next save
/// does not overwrite it; the warning is shown in the GUI.
pub fn load() -> (Config, Option<String>) {
    let Some(path) = config_path() else {
        return (
            Config::default(),
            Some("No config directory found; settings will not be saved.".into()),
        );
    };
    match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(config) => (config, None),
            Err(error) => {
                let backup = path.with_extension("json.bak");
                let _ = fs::rename(&path, &backup);
                let warning = format!(
                    "Could not read the config ({error}). The old file was moved to {}.",
                    backup.display()
                );
                (Config::default(), Some(warning))
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => (Config::default(), None),
        Err(error) => (
            Config::default(),
            Some(format!("Could not read {}: {error}", path.display())),
        ),
    }
}

/// Writes via a temporary file so an interrupted write cannot corrupt the config.
pub fn save(config: &Config) -> io::Result<()> {
    let path = config_path().ok_or_else(|| io::Error::other("no config directory"))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, json)?;
    fs::rename(&temp, &path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::{CountMode, TimeFormat};

    #[test]
    fn config_roundtrips_through_json() {
        let config = Config {
            bindings: vec![Binding {
                variable: "timer1".into(),
                ..Binding::default()
            }],
            ..Config::default()
        };
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&json).unwrap(), config);
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(config, Config::default());
    }

    #[test]
    fn config_from_the_button_text_version_still_loads() {
        let json = r#"{"connection":{},"bindings":[{"channel":2,"layer":5,
            "mode":"Up","format":"HhMmSs","target":{"page":1,"row":0,"column":3}}]}"#;
        let config: Config = serde_json::from_str(json).unwrap();

        let binding = &config.bindings[0];
        assert_eq!((binding.channel, binding.layer), (2, 5));
        assert_eq!(binding.mode, CountMode::Up);
        assert_eq!(binding.format, TimeFormat::HhMmSs);
        assert_eq!(binding.variable, "");
        assert!(binding.show_name && binding.show_time);
    }
}
