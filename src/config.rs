use directories::ProjectDirs;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use seq_macro::seq;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Trigger {
    pub name: String,
    pub modifiers: Vec<String>,
    pub key: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn dark(self, appearance: gpui::WindowAppearance) -> bool {
        match self {
            Self::System => matches!(
                appearance,
                gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark
            ),
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_voice_model")]
    pub voice_model: String,
    #[serde(default)]
    pub mic: String,
    #[serde(default = "default_true")]
    pub auto_paste: bool,
    #[serde(default = "default_true")]
    pub screenshot: bool,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default = "default_triggers", deserialize_with = "de_shortcuts")]
    pub shortcuts: [Trigger; 3],
}

pub const MUSE_BASE_URL: &str = "https://api.meta.ai/v1";
pub const MUSE_MODEL: &str = "muse-spark-1.3";
pub const VOICE_MODEL: &str = "muse-voice-transcribe-1.0";

fn default_voice_model() -> String {
    VOICE_MODEL.into()
}

fn default_model() -> String {
    MUSE_MODEL.into()
}

fn default_true() -> bool {
    true
}

fn default_triggers() -> [Trigger; 3] {
    [
        ("Dictate", ["CONTROL", "ALT"], "KeyQ"),
        ("Ask", ["CONTROL", "ALT"], "KeyW"),
        ("Helper", ["CONTROL", "ALT"], "KeyE"),
    ]
    .map(|(name, modifiers, key)| Trigger {
        name: name.into(),
        modifiers: modifiers.into_iter().map(str::to_owned).collect(),
        key: key.into(),
    })
}

fn normalize_triggers(list: Vec<Trigger>) -> [Trigger; 3] {
    let mut out = default_triggers();
    for t in list {
        let slot = match t.name.as_str() {
            "Dictate" => 0,
            "Ask" => 1,
            "Helper" => 2,
            _ => continue,
        };
        out[slot] = t;
    }
    out
}

fn de_shortcuts<'de, D>(deserializer: D) -> Result<[Trigger; 3], D::Error>
where
    D: serde::Deserializer<'de>,
{
    Vec::<Trigger>::deserialize(deserializer).map(normalize_triggers)
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            model: default_model(),
            voice_model: default_voice_model(),
            mic: String::new(),
            auto_paste: default_true(),
            screenshot: default_true(),
            theme: Theme::System,
            shortcuts: default_triggers(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        let dirs = ProjectDirs::from("", "", "trigger").expect("app data dir");
        let mut dir = dirs.config_dir().to_path_buf();
        if dir.file_name().is_some_and(|n| n == "config") {
            dir.pop();
        }
        dir.join("config.toml")
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn save(&self, path: &PathBuf) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        fs::write(
            path,
            toml::to_string_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())
    }

    pub fn effective_key(&self) -> String {
        if !self.api_key.is_empty() {
            return self.api_key.clone();
        }
        std::env::var("MODEL_API_KEY").unwrap_or_default()
    }

    pub fn to_hotkey(&self, trigger: &Trigger) -> Result<HotKey, String> {
        let modifiers = parse_modifiers(&trigger.modifiers)?;
        let code = parse_code(&trigger.key)?;
        Ok(HotKey::new(
            (!modifiers.is_empty()).then_some(modifiers),
            code,
        ))
    }
}

pub fn display_hotkey(trigger: &Trigger) -> String {
    let mut parts: Vec<String> = trigger
        .modifiers
        .iter()
        .map(|m| match m.to_ascii_uppercase().as_str() {
            "CONTROL" | "CTRL" => "Ctrl".into(),
            "ALT" => "Alt".into(),
            "SHIFT" => "Shift".into(),
            "SUPER" | "WIN" | "META" => if cfg!(target_os = "macos") {
                "Cmd"
            } else {
                "Super"
            }
            .into(),
            other => other.into(),
        })
        .collect();
    parts.push(if let Some(ch) = trigger.key.strip_prefix("Key") {
        ch.to_string()
    } else if let Some(d) = trigger.key.strip_prefix("Digit") {
        d.to_string()
    } else {
        trigger.key.clone()
    });
    parts.join(" + ")
}

pub fn capture_hotkey(name: &str, stroke: &gpui::Keystroke) -> Option<Trigger> {
    let modifiers = stroke.modifiers;
    let mut names = Vec::new();
    for (pressed, name) in [
        (modifiers.control, "CONTROL"),
        (modifiers.alt, "ALT"),
        (modifiers.shift, "SHIFT"),
        (modifiers.platform, "SUPER"),
    ] {
        if pressed {
            names.push(name.to_owned());
        }
    }
    if modifiers.function {
        return None;
    }
    let key = stroke.key.to_ascii_uppercase();
    let key = match key.as_str() {
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_uppercase() => format!("Key{key}"),
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_digit() => format!("Digit{key}"),
        "SPACE" => "Space".into(),
        _ if parse_code(&key).is_ok() => key,
        _ => return None,
    };
    Some(Trigger {
        name: name.into(),
        modifiers: names,
        key,
    })
}

fn parse_modifiers(names: &[String]) -> Result<Modifiers, String> {
    let mut mods = Modifiers::empty();
    for name in names {
        match name.to_ascii_uppercase().as_str() {
            "CONTROL" | "CTRL" => mods |= Modifiers::CONTROL,
            "ALT" => mods |= Modifiers::ALT,
            "SHIFT" => mods |= Modifiers::SHIFT,
            "SUPER" | "WIN" | "META" => mods |= Modifiers::SUPER,
            other => return Err(format!("unknown modifier `{other}`")),
        }
    }
    Ok(mods)
}

fn parse_code(name: &str) -> Result<Code, String> {
    seq!(C in 'A'..='Z' {
        if name == stringify!(Key~C) {
            return Ok(Code::Key~C);
        }
    });
    seq!(C in '0'..='9' {
        if name == stringify!(Digit~C) {
            return Ok(Code::Digit~C);
        }
    });
    seq!(N in 1..=12 {
        if name == stringify!(F~N) {
            return Ok(Code::F~N);
        }
    });
    if name == "Space" {
        return Ok(Code::Space);
    }
    Err(format!("unknown key `{name}`"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_missing_config_does_not_create_a_file() {
        let path = std::env::temp_dir().join(format!(
            "trigger-config-test-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let config = Config::load(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(config.theme, Theme::System);
    }

    #[test]
    fn legacy_config_keeps_credentials_and_uses_new_shortcuts() {
        let config: Config = toml::from_str(
            r#"
api_key = "example"
model = "muse-spark-1.3"
base_url = "https://api.meta.ai/v1"
[[triggers]]
name = "Screen"
key = "KeyQ"
modifiers = ["CONTROL", "ALT"]
capture = "screenshot"
"#,
        )
        .unwrap();
        assert_eq!(config.api_key, "example");
        assert_eq!(config.model, "muse-spark-1.3");
        assert_eq!(config.voice_model, VOICE_MODEL);
        assert_eq!(config.theme, Theme::System);
        assert_eq!(config.shortcuts[0].key, "KeyQ");
        assert_eq!(config.shortcuts[1].key, "KeyW");
        assert_eq!(config.shortcuts[2].name, "Helper");
        assert_eq!(config.shortcuts[2].key, "KeyE");
        assert_eq!(config.shortcuts[2].modifiers, ["CONTROL", "ALT"]);
        assert!(config.auto_paste);
        assert!(config.screenshot);
    }

    #[test]
    fn old_two_shortcut_configs_gain_helper() {
        let config: Config = toml::from_str(
            r#"
model = "muse-spark-1.3"
[[shortcuts]]
name = "Dictate"
key = "KeyQ"
modifiers = ["CONTROL", "ALT"]
[[shortcuts]]
name = "Ask"
key = "KeyW"
modifiers = ["CONTROL", "ALT"]
"#,
        )
        .unwrap();
        assert_eq!(config.shortcuts.len(), 3);
        assert_eq!(config.shortcuts[0].key, "KeyQ");
        assert_eq!(config.shortcuts[1].key, "KeyW");
        assert_eq!(config.shortcuts[2].name, "Helper");
        assert_eq!(config.shortcuts[2].key, "KeyE");
        assert_eq!(config.shortcuts[2].modifiers, ["CONTROL", "ALT"]);
    }

    #[test]
    fn theme_overrides_system_appearance() {
        use gpui::WindowAppearance::{Dark, Light};
        assert!(Theme::System.dark(Dark));
        assert!(!Theme::System.dark(Light));
        assert!(!Theme::Light.dark(Dark));
        assert!(Theme::Dark.dark(Light));
    }

    #[test]
    fn independent_models_and_theme_round_trip() {
        let config = Config {
            voice_model: "muse-voice-transcribe-1.0".into(),
            model: "muse-spark-1.3-contributor".into(),
            mic: "Test Mic".into(),
            auto_paste: false,
            screenshot: false,
            theme: Theme::Dark,
            ..Default::default()
        };
        let saved = toml::to_string(&config).unwrap();
        let loaded: Config = toml::from_str(&saved).unwrap();
        assert_eq!(loaded.voice_model, config.voice_model);
        assert_eq!(loaded.model, config.model);
        assert_eq!(loaded.mic, config.mic);
        assert_eq!(loaded.auto_paste, config.auto_paste);
        assert_eq!(loaded.screenshot, config.screenshot);
        assert_eq!(loaded.theme, config.theme);
    }

    #[test]
    fn captures_shortcuts_like_the_settings_recorder() {
        let stroke = gpui::Keystroke {
            modifiers: gpui::Modifiers {
                control: true,
                alt: true,
                ..Default::default()
            },
            key: "q".into(),
            key_char: None,
        };
        let shortcut = capture_hotkey("Dictate", &stroke).unwrap();
        assert_eq!(shortcut.key, "KeyQ");
        assert_eq!(shortcut.modifiers, ["CONTROL", "ALT"]);
        assert!(Config::default().to_hotkey(&shortcut).is_ok());
        let bare = gpui::Keystroke {
            modifiers: Default::default(),
            key: "f12".into(),
            key_char: None,
        };
        assert_eq!(capture_hotkey("Ask", &bare).unwrap().key, "F12");
        let unsupported = gpui::Keystroke {
            key: "tab".into(),
            ..bare
        };
        assert!(capture_hotkey("Ask", &unsupported).is_none());
    }
}
