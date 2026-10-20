//! A local, append-only record of what Orbit wrote.
//!
//! History is a convenience, never a source of truth: the files are on disk and
//! the log is only a way to find them again. That is why it is a JSONL file
//! rather than a database — appending one line cannot corrupt the earlier ones,
//! and a line that is truncated by a crash is skipped when the file is read.
//!
//! Nothing here leaves the machine, and nothing is uploaded. The file lives
//! under the platform's state directory (`~/.local/state/orbit` on Linux).

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// How many entries a compaction keeps.
pub const KEPT: usize = 500;

/// How large the file may grow before it is compacted.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// One finished job, as the tray and the settings window show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// Unix milliseconds when the job finished.
    pub at: u64,
    /// Human label, such as `Convert to WEBP` or `Web ready`.
    pub label: String,
    /// The petal reference that ran, for re-running it later.
    pub action: String,
    /// What was worked on.
    #[serde(default)]
    pub sources: Vec<PathBuf>,
    /// What was published. Empty for a failure or a cancellation.
    #[serde(default)]
    pub outputs: Vec<PathBuf>,
    /// False when the job failed or was cancelled.
    pub ok: bool,
    /// The engine's last words, for a failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl HistoryEntry {
    /// Entry for a job that just finished.
    #[must_use]
    pub fn now(
        label: impl Into<String>,
        action: impl Into<String>,
        sources: Vec<PathBuf>,
        outputs: Vec<PathBuf>,
        ok: bool,
        message: Option<String>,
    ) -> Self {
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(0));
        Self {
            at,
            label: label.into(),
            action: action.into(),
            sources,
            outputs,
            ok,
            message,
        }
    }

    /// The first file to reveal, or the first source for a failure.
    #[must_use]
    pub fn anchor(&self) -> Option<&Path> {
        self.outputs
            .first()
            .or_else(|| self.sources.first())
            .map(PathBuf::as_path)
    }

    /// A one-line menu label: `photo (converted).webp` or the action name.
    #[must_use]
    pub fn menu_label(&self) -> String {
        self.anchor().map_or_else(
            || self.label.clone(),
            |path| {
                path.file_name().map_or_else(
                    || path.display().to_string(),
                    |name| name.to_string_lossy().into_owned(),
                )
            },
        )
    }
}

/// The file history is kept in.
#[derive(Debug, Clone)]
pub struct HistoryStore {
    path: PathBuf,
}

impl HistoryStore {
    /// A store at an explicit path, which is what tests and `--history` use.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Where this installation keeps its history.
    ///
    /// `ORBIT_HISTORY` overrides it, so the CLI and a test can run without
    /// touching the real log.
    #[must_use]
    pub fn default_path() -> PathBuf {
        if let Some(path) = std::env::var_os("ORBIT_HISTORY") {
            return PathBuf::from(path);
        }
        if cfg!(target_os = "windows") {
            if let Some(base) = std::env::var_os("LOCALAPPDATA") {
                return PathBuf::from(base).join("Orbit").join("history.jsonl");
            }
        }
        if cfg!(target_os = "macos") {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home)
                    .join("Library/Application Support/Orbit")
                    .join("history.jsonl");
            }
        }
        if let Some(state) = std::env::var_os("XDG_STATE_HOME") {
            return PathBuf::from(state).join("orbit").join("history.jsonl");
        }
        std::env::var_os("HOME").map_or_else(
            || std::env::temp_dir().join("orbit-history.jsonl"),
            |home| {
                PathBuf::from(home)
                    .join(".local/state/orbit")
                    .join("history.jsonl")
            },
        )
    }

    /// The file this store reads and writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Add one entry, then compact if the file has grown too large.
    ///
    /// # Errors
    ///
    /// Fails when the directory or the file cannot be written. A caller that
    /// cannot record history should still keep the file it just produced, so
    /// this is reported rather than propagated as a job failure.
    pub fn append(&self, entry: &HistoryEntry) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{line}")?;
        drop(file);

        let too_big = fs::metadata(&self.path).is_ok_and(|meta| meta.len() > MAX_BYTES);
        if too_big {
            // Keep the newest entries. A compaction failure must not undo the
            // append, which has already succeeded.
            let _ = self.compact(KEPT);
        }
        Ok(())
    }

    /// The newest `limit` entries, newest first.
    ///
    /// A malformed line is skipped: half a line from a crash should cost one
    /// entry, not the whole log.
    #[must_use]
    pub fn load(&self, limit: usize) -> Vec<HistoryEntry> {
        let Ok(file) = fs::File::open(&self.path) else {
            return Vec::new();
        };
        let mut entries: Vec<HistoryEntry> = BufReader::new(file)
            .lines()
            .map_while(Result::ok)
            .filter(|line| !line.trim().is_empty())
            .filter_map(|line| serde_json::from_str(&line).ok())
            .collect();
        if entries.len() > limit {
            entries.drain(..entries.len() - limit);
        }
        entries.reverse();
        entries
    }

    /// Remove every entry.
    ///
    /// # Errors
    ///
    /// Fails when the file exists and cannot be removed.
    pub fn clear(&self) -> std::io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    /// Rewrite the file with only the newest `keep` entries.
    ///
    /// Written to a sibling and renamed into place, so a crash cannot leave a
    /// half-compacted log behind.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or replaced.
    pub fn compact(&self, keep: usize) -> std::io::Result<()> {
        let entries = self.load(keep);
        let Some(parent) = self.path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(parent)?;
        let temp = self.path.with_extension("jsonl.new");
        {
            let mut file = fs::File::create(&temp)?;
            // `load` returns newest first; the file is written oldest first so
            // the next append stays chronological.
            for entry in entries.iter().rev() {
                let line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
                writeln!(file, "{line}")?;
            }
        }
        replace_file(&temp, &self.path)
    }
}

/// Replace `to` with `from`.
///
/// Unix renames over an existing file. Windows refuses to, so the destination
/// is removed first. A crash in that gap costs the existing log, which is a
/// convenience; the files it names are still on disk.
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    if to.exists() {
        fs::remove_file(to)?;
    }
    fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "orbit-history-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos())
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn entry(label: &str) -> HistoryEntry {
        HistoryEntry::now(
            label,
            "convert",
            vec![PathBuf::from("/src/photo.png")],
            vec![PathBuf::from("/src/photo (converted).webp")],
            true,
            None,
        )
    }

    #[test]
    fn entries_are_appended_and_read_newest_first() {
        let dir = TempDir::new("roundtrip");
        let store = HistoryStore::new(dir.0.join("history.jsonl"));
        store.append(&entry("first")).unwrap();
        store.append(&entry("second")).unwrap();

        let entries = store.load(10);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "second");
        assert_eq!(entries[1].label, "first");

        let limited = store.load(1);
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].label, "second");
    }

    #[test]
    fn a_malformed_line_costs_only_itself() {
        let dir = TempDir::new("malformed");
        let path = dir.0.join("history.jsonl");
        let store = HistoryStore::new(&path);
        store.append(&entry("good")).unwrap();
        fs::write(
            &path,
            format!("{}\n{{not json\n", fs::read_to_string(&path).unwrap()),
        )
        .unwrap();

        let entries = store.load(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "good");
    }

    #[test]
    fn compaction_keeps_the_newest_and_stays_appendable() {
        let dir = TempDir::new("compact");
        let store = HistoryStore::new(dir.0.join("history.jsonl"));
        for index in 0..6 {
            store.append(&entry(&format!("job {index}"))).unwrap();
        }
        store.compact(2).unwrap();
        assert_eq!(store.load(10).len(), 2);
        assert_eq!(store.load(1)[0].label, "job 5");

        store.append(&entry("after")).unwrap();
        let entries = store.load(10);
        assert_eq!(entries[0].label, "after");
        assert_eq!(entries[1].label, "job 5");
    }

    #[test]
    fn clearing_a_missing_file_is_not_an_error() {
        let dir = TempDir::new("clear");
        let store = HistoryStore::new(dir.0.join("never-written.jsonl"));
        store.clear().unwrap();
        assert!(store.load(10).is_empty(), "clearing left entries behind");
    }

    #[test]
    fn the_menu_label_names_the_result_not_the_full_path() {
        let entry = entry("Convert to WebP");
        assert_eq!(entry.menu_label(), "photo (converted).webp");
        assert_eq!(
            entry.anchor().unwrap(),
            Path::new("/src/photo (converted).webp")
        );
    }
}
