//! The small amount of state Orbit remembers between runs.
//!
//! Today that is one thing: the global shortcut. It lives in a plain JSON file
//! beside the app's other configuration, because the alternative — a shortcut
//! that resets on every launch, or one that can only be changed by editing a
//! source file — is exactly the sort of thing that makes a background utility
//! annoying enough to uninstall.
//!
//! The file is written atomically, as the job outputs are: a crash mid-write
//! must not leave a half-written settings file behind, or the next launch would
//! silently fall back to the default with no way to tell why.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The shortcut registered on a fresh install.
pub const DEFAULT_HOTKEY: &str = "CmdOrCtrl+Shift+Space";

/// The shortcuts the tray offers, as (accelerator, how to show it).
///
/// A short list rather than a key-capture recorder: every entry is a combination
/// with a modifier, so it is unlikely to collide with an application's own
/// shortcut, and none of them need a keyboard-grab dialog that a background app
/// has no good way to present.
pub const PRESETS: &[(&str, &str)] = &[
    ("CmdOrCtrl+Shift+Space", "Ctrl/Cmd + Shift + Space"),
    ("CmdOrCtrl+Alt+Space", "Ctrl/Cmd + Alt + Space"),
    ("CmdOrCtrl+Shift+O", "Ctrl/Cmd + Shift + O"),
    ("Alt+Shift+Space", "Alt + Shift + Space"),
];

/// What the tray should show for the shortcut that is running now.
#[derive(Debug, Clone, Serialize)]
pub struct HotkeyStatus {
    /// The accelerator in force, or null when the user turned the shortcut off.
    pub accelerator: Option<String>,
    /// The same thing written the way a person would say it.
    pub label: String,
    /// Whether the registration actually took.
    pub registered: bool,
    /// One line explaining a registration that could not work, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl HotkeyStatus {
    /// Turned off on purpose.
    #[must_use]
    pub fn off() -> Self {
        Self {
            accelerator: None,
            label: "Off".to_owned(),
            registered: false,
            note: None,
        }
    }
}

/// The persisted settings document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// The global shortcut.
    ///
    /// An absent field means the default, and an explicit `null` means the user
    /// turned the shortcut off — two states that deserve to be distinguishable
    /// in the file rather than collapsed into one.
    #[serde(default = "default_hotkey")]
    pub hotkey: Option<String>,
}

// The signature is fixed at the field's type, which `Option`-wraps the value so
// that a missing field and an explicit `null` stay distinguishable.
#[allow(clippy::unnecessary_wraps)]
fn default_hotkey() -> Option<String> {
    Some(DEFAULT_HOTKEY.to_owned())
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
        }
    }
}

impl Settings {
    /// Read the settings, falling back to the defaults.
    ///
    /// A file that cannot be read or parsed is reported and then ignored: a
    /// corrupt settings file should cost the user their shortcut choice, not the
    /// ability to start Orbit.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        match serde_json::from_str(&text) {
            Ok(settings) => settings,
            Err(error) => {
                eprintln!(
                    "orbit: {} is not readable ({error}); using the defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Write the settings, replacing the file in one step.
    ///
    /// # Errors
    /// Fails when the directory cannot be created or the file cannot be written
    /// or renamed into place.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let temporary = path.with_extension("json.new");
        fs::write(&temporary, format!("{text}\n"))?;
        fs::rename(&temporary, path)
    }
}

/// The label for an accelerator, in the words the presets use.
#[must_use]
pub fn label_for(accelerator: &str) -> String {
    PRESETS
        .iter()
        .find(|(words, _)| *words == accelerator)
        .map_or_else(|| accelerator.to_owned(), |(_, label)| (*label).to_owned())
}

/// Where the settings file lives, given the app's configuration directory.
#[must_use]
pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("orbit-settings-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_file_means_the_default_shortcut() {
        let dir = scratch("missing");
        let settings = Settings::load(&settings_path(&dir));
        assert_eq!(settings.hotkey.as_deref(), Some(DEFAULT_HOTKEY));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_choice_survives_a_round_trip() {
        let dir = scratch("round-trip");
        let path = settings_path(&dir);
        let settings = Settings {
            hotkey: Some("Ctrl+Alt+O".to_owned()),
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).hotkey.as_deref(), Some("Ctrl+Alt+O"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn turning_the_shortcut_off_is_not_the_same_as_never_having_chosen() {
        let dir = scratch("off");
        let path = settings_path(&dir);
        Settings { hotkey: None }.save(&path).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("null"), "{text}");
        assert_eq!(Settings::load(&path).hotkey, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_corrupt_file_falls_back_instead_of_failing_to_start() {
        let dir = scratch("corrupt");
        let path = settings_path(&dir);
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(
            Settings::load(&path).hotkey.as_deref(),
            Some(DEFAULT_HOTKEY)
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn presets_all_have_labels() {
        for (accelerator, _) in PRESETS {
            assert_ne!(label_for(accelerator), *accelerator);
        }
        assert_eq!(label_for("F13"), "F13");
    }
}
