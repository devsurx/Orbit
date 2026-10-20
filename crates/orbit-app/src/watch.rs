//! Watch folders: turn a folder into a standing instruction.
//!
//! The scanner is deliberately simple. A poll every couple of seconds, a set of
//! paths already seen, and a minimum age so a file that is still being copied is
//! not grabbed halfway through. No filesystem watcher library, because the
//! difference between "2 seconds late" and "instantly" does not matter for a
//! batch converter, and a poll cannot miss an event the way an inotify queue can.
//!
//! Two loops are avoided explicitly:
//!
//! * the first scan of a folder only *memorises* what is there, so enabling a
//!   watch does not convert the existing contents;
//! * anything Orbit itself published is skipped, so an output written beside its
//!   source in a watched folder is not converted again forever.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use orbit_jobs::{Pick, PickKind, resolve_pick};
use tauri::{AppHandle, Manager, Runtime};

use crate::settings::WatchFolder;
use crate::{Jobs, Orbit, enqueue};

/// How often the folders are scanned.
pub const POLL: Duration = Duration::from_secs(2);

/// How old a file must be before it is considered complete.
pub const MIN_AGE: Duration = Duration::from_secs(2);

/// How deep a watch reaches below the folder it was pointed at.
const MAX_DEPTH: usize = 3;

/// The files a folder currently contains, recursively, ignoring hidden ones.
#[must_use]
pub fn walk(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    walk_into(root, 0, &mut found);
    found
}

fn walk_into(root: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let hidden = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.'));
        if hidden {
            // Orbit's own scratch files, and editor swap files, live here.
            continue;
        }
        if path.is_dir() {
            walk_into(&path, depth + 1, found);
        } else if path.is_file() {
            found.push(path);
        }
    }
}

/// Per-folder memory of what has already been considered.
#[derive(Debug, Default)]
pub struct Scanner {
    primed: BTreeSet<PathBuf>,
    seen: BTreeSet<PathBuf>,
}

impl Scanner {
    /// Scan one folder and return the files that are new and old enough.
    ///
    /// The first call primes the folder and returns nothing; that is what keeps
    /// "watch this folder" from being "convert everything in this folder".
    pub fn poll(&mut self, dir: &Path, min_age: Duration, now: SystemTime) -> Vec<PathBuf> {
        let files = walk(dir);
        if self.primed.insert(dir.to_path_buf()) {
            self.seen.extend(files);
            return Vec::new();
        }

        let mut fresh = Vec::new();
        for path in files {
            if self.seen.contains(&path) {
                continue;
            }
            let age = fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .unwrap_or_default();
            if age >= min_age {
                // Only claim it once it is old enough to be complete. Marking a
                // file seen while it is still too young would silently drop a
                // copy that was half-written during the previous poll.
                self.seen.insert(path.clone());
                fresh.push(path);
            }
        }
        fresh
    }

    /// Forget a folder, so re-adding it primes instead of replaying.
    pub fn forget(&mut self, dir: &Path) {
        self.primed.remove(dir);
        self.seen.retain(|path| !path.starts_with(dir));
    }
}

/// Start the watch loop.
///
/// It reads the settings on every tick, so adding, pausing or removing a folder
/// in the settings window takes effect without restarting Orbit.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    let spawned = std::thread::Builder::new()
        .name("orbit-watch".to_owned())
        .spawn(move || {
            let mut scanners: std::collections::BTreeMap<PathBuf, Scanner> =
                std::collections::BTreeMap::new();
            loop {
                let Some(orbit) = app.try_state::<Orbit>() else {
                    return;
                };
                let Some(settings) = orbit.settings.lock().ok().map(|s| s.clone()) else {
                    std::thread::sleep(POLL);
                    continue;
                };
                for rule in settings.watch.iter().filter(|rule| rule.enabled) {
                    let scanner = scanners.entry(rule.path.clone()).or_default();
                    let fresh = scanner.poll(&rule.path, MIN_AGE, SystemTime::now());
                    for path in fresh {
                        if is_orbit_output(&app, &path) {
                            continue;
                        }
                        submit(&app, rule, &path, settings.output_dir.as_deref());
                    }
                }

                // A removed folder should not keep its stale path set forever.
                let live: BTreeSet<&PathBuf> =
                    settings.watch.iter().map(|rule| &rule.path).collect();
                scanners.retain(|path, scanner| {
                    if live.contains(path) {
                        true
                    } else {
                        scanner.forget(path);
                        false
                    }
                });

                std::thread::sleep(POLL);
            }
        });

    if let Err(error) = spawned {
        eprintln!("orbit: watch folders are off — the thread could not start ({error})");
    }
}

/// Whether Orbit wrote this path a moment ago.
fn is_orbit_output<R: Runtime>(app: &AppHandle<R>, path: &Path) -> bool {
    app.try_state::<Orbit>()
        .and_then(|orbit| {
            orbit
                .written
                .lock()
                .ok()
                .map(|written| written.contains(path))
        })
        .unwrap_or(false)
}

/// Resolve a folder's rule for one file and queue it.
fn submit<R: Runtime>(
    app: &AppHandle<R>,
    rule: &WatchFolder,
    path: &Path,
    output_dir: Option<&Path>,
) {
    let Some(orbit) = app.try_state::<Orbit>() else {
        return;
    };
    let Some(kind) = PickKind::from_wire(&rule.kind) else {
        eprintln!(
            "orbit: watch {} has an unknown rule kind",
            rule.path.display()
        );
        return;
    };
    let pick = Pick {
        kind,
        reference: rule.reference.clone(),
        defaults: rule.options.clone(),
    };
    let resolution = match resolve_pick(
        &orbit.registry,
        &orbit.statuses(),
        &pick,
        &[path.to_path_buf()],
    ) {
        Ok(resolution) => resolution,
        Err(error) => {
            eprintln!(
                "orbit: watch could not resolve {} — {error}",
                path.display()
            );
            return;
        }
    };
    if resolution.requests.is_empty() {
        eprintln!(
            "orbit: watch skipped {} — {}",
            path.display(),
            resolution.first_reason().unwrap_or("no suitable action")
        );
        return;
    }
    let mut requests = resolution.requests;
    for request in &mut requests {
        request.output_dir = output_dir.map(Path::to_path_buf);
    }

    let Some(jobs) = app.try_state::<Jobs>() else {
        return;
    };
    let label = format!("Watch: {}", rule.reference);
    match enqueue(&jobs, &label, &rule.reference, requests) {
        Ok(queued) => {
            eprintln!(
                "orbit: watch {} → {} queued {} job(s)",
                rule.path.display(),
                path.display(),
                queued
            );
        }
        Err(error) => eprintln!("orbit: watch could not queue {} — {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "orbit-watch-{label}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |elapsed| elapsed.as_nanos())
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn the_first_scan_primes_instead_of_converting_the_backlog() {
        let dir = TempDir::new("prime");
        fs::write(dir.0.join("old.png"), b"old").unwrap();
        let mut scanner = Scanner::default();
        assert!(
            scanner
                .poll(&dir.0, Duration::ZERO, SystemTime::now())
                .is_empty(),
            "the first scan only memorises"
        );
        // The same file is not "new" on the next scan either.
        assert!(
            scanner
                .poll(&dir.0, Duration::ZERO, SystemTime::now())
                .is_empty(),
            "a primed folder has no backlog"
        );
    }

    #[test]
    fn a_file_that_appears_later_is_returned_once() {
        let dir = TempDir::new("fresh");
        let mut scanner = Scanner::default();
        scanner.poll(&dir.0, Duration::ZERO, SystemTime::now());
        fs::write(dir.0.join("new.png"), b"new").unwrap();

        let fresh = scanner.poll(&dir.0, Duration::ZERO, SystemTime::now());
        assert_eq!(fresh, [dir.0.join("new.png")]);
        assert!(
            scanner
                .poll(&dir.0, Duration::ZERO, SystemTime::now())
                .is_empty(),
            "a file is only new once"
        );
    }

    #[test]
    fn a_file_that_is_still_being_written_waits() {
        let dir = TempDir::new("young");
        let mut scanner = Scanner::default();
        scanner.poll(&dir.0, Duration::ZERO, SystemTime::now());
        fs::write(dir.0.join("partial.png"), b"half").unwrap();

        // With a one-hour minimum age, the file is not ready.
        assert!(
            scanner
                .poll(&dir.0, Duration::from_secs(3600), SystemTime::now())
                .is_empty(),
            "a file younger than the minimum age is not ready"
        );
        // It is reconsidered on the next scan, and picked up once it is old
        // enough. A copy that was still running is not lost forever.
        assert_eq!(
            scanner.poll(&dir.0, Duration::ZERO, SystemTime::now()),
            [dir.0.join("partial.png")]
        );
    }

    #[test]
    fn nested_and_hidden_files_are_handled() {
        let dir = TempDir::new("nested");
        let mut scanner = Scanner::default();
        scanner.poll(&dir.0, Duration::ZERO, SystemTime::now());
        fs::create_dir_all(dir.0.join("sub")).unwrap();
        fs::write(dir.0.join("sub").join("deep.png"), b"deep").unwrap();
        fs::write(dir.0.join(".hidden.png"), b"hidden").unwrap();

        let fresh = scanner.poll(&dir.0, Duration::ZERO, SystemTime::now());
        assert_eq!(fresh, [dir.0.join("sub").join("deep.png")]);
    }
}
