//! Engine selection and local availability detection.
//!
//! Availability is injectable so the registry can be tested without installing
//! `FFmpeg`, and so the UI can explain *why* a petal is greyed out.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::model::{Engine, Id, Progress};

/// Platform key used by `engines.json`, derived at compile time.
#[must_use]
pub const fn host_platform() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "macos-arm64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "windows-x64"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "windows-arm64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "linux-x64"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "linux-arm64"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
    )))]
    {
        "unknown"
    }
}

/// What Orbit found on this machine for one engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineStatus {
    /// Engine id.
    pub id: Id,
    /// True when the engine can be used right now.
    pub available: bool,
    /// Executable that was resolved, absent for built-in engines.
    pub binary: Option<PathBuf>,
    /// Why it is unavailable, phrased for the UI.
    pub reason: Option<String>,
    /// Progress protocol the runner should expect.
    pub progress: Progress,
}

impl EngineStatus {
    /// Short sentence for a tooltip or the Engines settings page.
    #[must_use]
    pub fn status_line(&self, label: &str) -> String {
        if self.available {
            match &self.binary {
                Some(path) => format!("{label}: found at {}", path.display()),
                None => format!("{label}: built in"),
            }
        } else {
            format!(
                "{label}: {}",
                self.reason.as_deref().unwrap_or("not available")
            )
        }
    }
}

/// Locate an engine's executable on `PATH`, honouring the platform limits
/// declared in the registry.
#[must_use]
pub fn detect(engine: &Engine, platform: &str, path_var: Option<&str>) -> EngineStatus {
    if !engine.supports_platform(platform) {
        return EngineStatus {
            id: engine.id.clone(),
            available: false,
            binary: None,
            reason: Some(format!("not built for {platform}")),
            progress: engine.progress,
        };
    }

    if engine.binaries().is_empty() {
        return EngineStatus {
            id: engine.id.clone(),
            available: true,
            binary: None,
            reason: None,
            progress: engine.progress,
        };
    }

    let search_path = path_var.map_or_else(
        || std::env::var_os("PATH"),
        |value| Some(std::ffi::OsString::from(value)),
    );

    for name in engine.binaries() {
        if let Some(found) = which(name, search_path.as_deref()) {
            return EngineStatus {
                id: engine.id.clone(),
                available: true,
                binary: Some(found),
                reason: None,
                progress: engine.progress,
            };
        }
    }

    EngineStatus {
        id: engine.id.clone(),
        available: false,
        binary: None,
        reason: Some(format!("{} is not installed", engine.binaries()[0])),
        progress: engine.progress,
    }
}

/// Minimal `which(1)`: resolve `name` against a PATH-style variable.
fn which(name: &str, path_var: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }

    let mut dirs = std::env::split_paths(path_var?).collect::<Vec<_>>();
    if dirs.is_empty() {
        dirs = vec![PathBuf::from(".")];
    }

    #[cfg(windows)]
    let suffixes: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".EXE;.BAT;.CMD".to_owned())
        .split(';')
        .map(str::to_lowercase)
        .collect();
    #[cfg(not(windows))]
    let suffixes: Vec<String> = vec![String::new()];

    for dir in dirs {
        for suffix in &suffixes {
            let full = dir.join(format!("{name}{suffix}"));
            if full.is_file() {
                return Some(full);
            }
        }
    }
    None
}

/// Availability of every engine in a registry, keyed by id.
#[must_use]
pub fn detect_all(
    engines: &[Engine],
    platform: &str,
    path_var: Option<&str>,
) -> BTreeMap<Id, EngineStatus> {
    engines
        .iter()
        .map(|engine| (engine.id.clone(), detect(engine, platform, path_var)))
        .collect()
}

/// The set of engine ids that are usable right now.
#[must_use]
pub fn available_ids(statuses: &BTreeMap<Id, EngineStatus>) -> BTreeSet<Id> {
    statuses
        .iter()
        .filter(|(_, status)| status.available)
        .map(|(id, _)| id.clone())
        .collect()
}
