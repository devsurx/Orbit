//! The small amount of state Orbit remembers between runs.
//!
//! That is the shortcut, the Shift-drag switch, the output folder, the watch
//! rules and whether the welcome has been shown. It lives in a plain JSON file
//! beside the app's other configuration, because the alternative — settings
//! that reset on every launch, or ones that can only be changed by editing a
//! source file — is exactly the sort of thing that makes a background utility
//! annoying enough to uninstall.
//!
//! The file is written atomically, as the job outputs are: a crash mid-write
//! must not leave a half-written settings file behind, or the next launch would
//! silently fall back to the default with no way to tell why.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

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

/// One folder Orbit watches, and what to do with files that appear in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchFolder {
    /// Folder to scan.
    pub path: PathBuf,
    /// `conversion`, `action` or `preset`.
    pub kind: String,
    /// Format id, action id or chain id.
    pub reference: String,
    /// Option values applied to every file found here.
    #[serde(default)]
    pub options: BTreeMap<String, Value>,
    /// Whether the rule runs; the folder stays listed when it is paused.
    #[serde(default = "default_true")]
    pub enabled: bool,
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
    /// Folder every result goes to, or `None` for "beside the source".
    #[serde(default)]
    pub output_dir: Option<PathBuf>,
    /// Whether Shift-drag summons the ring.
    #[serde(default = "default_true")]
    pub trigger: bool,
    /// False until the first-run window has been dismissed.
    #[serde(default)]
    pub onboarded: bool,
    /// Folders watched for new files.
    #[serde(default)]
    pub watch: Vec<WatchFolder>,
}

// The signature is fixed at the field's type, which `Option`-wraps the value so
// that a missing field and an explicit `null` stay distinguishable.
#[allow(clippy::unnecessary_wraps)]
fn default_hotkey() -> Option<String> {
    Some(DEFAULT_HOTKEY.to_owned())
}

const fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey(),
            output_dir: None,
            trigger: true,
            onboarded: false,
            watch: Vec::new(),
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
        replace_file(&temporary, path)
    }
}

/// Replace `to` with `from`.
///
/// Unix renames over an existing file. Windows refuses to, so the destination
/// is removed first; the crash window that opens is a Windows-only trade for
/// not depending on a platform API just to save a few preferences.
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    if to.exists() {
        fs::remove_file(to)?;
    }
    fs::rename(from, to)
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
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).hotkey.as_deref(), Some("Ctrl+Alt+O"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn turning_the_shortcut_off_is_not_the_same_as_never_having_chosen() {
        let dir = scratch("off");
        let path = settings_path(&dir);
        Settings {
            hotkey: None,
            ..Settings::default()
        }
        .save(&path)
        .unwrap();

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
    fn watch_rules_survive_a_round_trip() {
        let dir = scratch("watch");
        let path = settings_path(&dir);
        let settings = Settings {
            watch: vec![WatchFolder {
                path: PathBuf::from("/home/surx/inbox"),
                kind: "conversion".to_owned(),
                reference: "webp".to_owned(),
                options: BTreeMap::from([("quality".to_owned(), serde_json::json!(70))]),
                enabled: true,
            }],
            output_dir: Some(PathBuf::from("/home/surx/out")),
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.watch, settings.watch);
        assert_eq!(loaded.output_dir, settings.output_dir);
        assert!(loaded.trigger);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn older_settings_files_gain_defaults_for_new_fields() {
        let dir = scratch("upgrade");
        let path = settings_path(&dir);
        fs::write(&path, r#"{ "hotkey": "Ctrl+Shift+O" }"#).unwrap();
        let loaded = Settings::load(&path);
        assert!(loaded.trigger);
        assert!(
            loaded.watch.is_empty(),
            "a new settings file starts with no watch rules"
        );
        assert!(!loaded.onboarded);
        assert_eq!(loaded.output_dir, None);
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
