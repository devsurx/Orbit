//! The worker pool.
//!
//! Jobs run on pool threads so the UI thread never blocks, and every job is a
//! child process this module supervises: spawn, read progress, wait, publish.
//!
//! Two behaviours matter more than throughput:
//!
//! * **A job can always be cancelled.** The worker polls the child instead of
//!   blocking on it, so a cancel lands within [`POLL`] even if the engine has
//!   gone quiet.
//! * **A job that fails leaves nothing on disk.** The plan's scratch file is
//!   removed by [`crate::output::OutputPlan`]'s drop, including on cancellation.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::Serialize;

use crate::native;
use crate::plan::{AuxFile, Backend, CommandSpec, JobRequest, ProgressMode, plan};

/// How long a worker waits between cancellation checks while a child runs.
const POLL: Duration = Duration::from_millis(80);

/// Cap on retained stderr, so a chatty engine cannot grow memory without bound.
const STDERR_LIMIT: u64 = 8 * 1024;

/// Companion executable used for duration probing, per `registry/engines.json`.
const PROBE: &str = "ffprobe";

/// Identifies a job for the life of the process.
pub type JobId = u64;

/// Where a job is in its life.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum JobState {
    /// Accepted, waiting for a worker.
    Queued,
    /// Under way. `percent` is absent when the engine cannot report progress.
    Running {
        /// Completion in the range 0-100, when it is known.
        #[serde(skip_serializing_if = "Option::is_none")]
        percent: Option<f64>,
    },
    /// Finished, published at `path`.
    Done {
        /// Where the result was written.
        path: PathBuf,
    },
    /// Finished, publishing a numbered run (PDF pages, video frames).
    DoneSeries {
        /// Where every result was written, in order.
        paths: Vec<PathBuf>,
    },
    /// Gave up. `message` is engine stderr or a planning error.
    Failed {
        /// Human-readable reason.
        message: String,
    },
    /// Stopped because the user asked.
    Cancelled,
}

/// A progress report, pushed to whoever is listening.
#[derive(Debug, Clone, Serialize)]
pub struct JobEvent {
    /// Which job this is about.
    pub id: JobId,
    /// Name shown in the UI, such as `Convert to WEBP`.
    pub label: String,
    /// What just happened.
    #[serde(flatten)]
    pub state: JobState,
}

/// A unit of work: what to run, and what to call it.
#[derive(Debug, Clone)]
pub struct Job {
    /// Assigned by the caller.
    pub id: JobId,
    /// Name shown in the UI.
    pub label: String,
    /// The resolved request.
    pub request: JobRequest,
}

/// Callback invoked on a pool thread for every state change.
type Emitter = Arc<dyn Fn(JobEvent) + Send + Sync>;

/// Cancel flags for jobs that are currently running.
type Cancels = Arc<Mutex<HashMap<JobId, Arc<AtomicBool>>>>;

/// A fixed pool of threads that turn [`Job`]s into files.
pub struct Runner {
    sender: Option<Sender<Job>>,
    workers: Vec<JoinHandle<()>>,
    cancels: Cancels,
    closed: Arc<AtomicBool>,
}

impl Runner {
    /// Start a pool. `emit` is called from pool threads, so it must not block
    /// for long; the shell forwards events and returns.
    ///
    /// A worker count of zero is treated as one, because a pool that never runs
    /// anything is never what the caller meant.
    #[must_use]
    pub fn new(workers: usize, emit: impl Fn(JobEvent) + Send + Sync + 'static) -> Self {
        let (sender, receiver) = mpsc::channel::<Job>();
        let queue = Arc::new(Mutex::new(receiver));
        let emit: Emitter = Arc::new(emit);
        let cancels: Cancels = Arc::new(Mutex::new(HashMap::new()));

        let workers = workers.max(1);
        let closed = Arc::new(AtomicBool::new(false));
        let handles = (0..workers)
            .map(|_| {
                let queue = Arc::clone(&queue);
                let emit = Arc::clone(&emit);
                let cancels = Arc::clone(&cancels);
                let closed = Arc::clone(&closed);
                thread::spawn(move || {
                    loop {
                        let next = {
                            let inbox = queue
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            inbox.recv()
                        };
                        match next {
                            // Shutting down: say so rather than starting work that
                            // nobody is waiting for any more.
                            Ok(job) if closed.load(Ordering::Relaxed) => {
                                report(&emit, &job, JobState::Cancelled);
                            }
                            Ok(job) => run_job(&job, &emit, &cancels, &closed),
                            // The sender is gone: the app is shutting down.
                            Err(_) => break,
                        }
                    }
                })
            })
            .collect();

        Self {
            sender: Some(sender),
            workers: handles,
            cancels,
            closed,
        }
    }

    /// Queue a job. Returns `false` once the pool has been shut down.
    pub fn submit(&self, job: Job) -> bool {
        self.sender
            .as_ref()
            .is_some_and(|sender| sender.send(job).is_ok())
    }

    /// Ask a running job to stop. The child is killed and its scratch file
    /// removed; a job that has not started yet is cancelled when it picks up.
    pub fn cancel(&self, id: JobId) {
        let flag = {
            let cancels = self
                .cancels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cancels.get(&id).map(Arc::clone)
        };
        if let Some(flag) = flag {
            flag.store(true, Ordering::Relaxed);
        }
    }

    /// Ask every running job to stop and wait for the threads to end.
    ///
    /// Queued work is abandoned, not started: a shutdown that runs the backlog
    /// first is a shutdown that appears to hang.
    pub fn shutdown(&mut self) {
        // Close first, then drop the sender. Workers drain whatever is already
        // in the queue, see the flag, and report it cancelled instead of
        // spawning a child for it.
        self.closed.store(true, Ordering::Relaxed);
        self.sender = None;
        let ids: Vec<JobId> = {
            let cancels = self
                .cancels
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cancels.keys().copied().collect()
        };
        for id in ids {
            self.cancel(id);
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Why a job did not produce a file.
enum Failure {
    /// The request could not become a command line.
    Plan(String),
    /// The engine exited non-zero.
    Exit(String),
    /// The built-in engine gave up.
    Native(String),
    /// The user cancelled.
    Cancelled,
    /// Spawning, preparing or publishing failed.
    Io(String),
}

fn run_job(job: &Job, emit: &Emitter, cancels: &Cancels, closed: &AtomicBool) {
    let flag = Arc::new(AtomicBool::new(false));
    {
        let mut cancels = cancels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cancels.insert(job.id, Arc::clone(&flag));
    }

    report(emit, job, JobState::Queued);

    // Registering the flag and reading `closed` in this order closes the window
    // where shutdown sweeps the cancel map just before this job appears in it:
    // either the sweep sees the flag, or this check sees the shutdown.
    let outcome = if closed.load(Ordering::Relaxed) || flag.load(Ordering::Relaxed) {
        Err(Failure::Cancelled)
    } else {
        execute(job, emit, &flag)
    };

    {
        let mut cancels = cancels
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cancels.remove(&job.id);
    }

    report(
        emit,
        job,
        match outcome {
            Ok(paths) if paths.len() == 1 => JobState::Done {
                path: paths.into_iter().next().unwrap_or_default(),
            },
            Ok(paths) => JobState::DoneSeries { paths },
            Err(Failure::Cancelled) => JobState::Cancelled,
            Err(
                Failure::Plan(message)
                | Failure::Exit(message)
                | Failure::Native(message)
                | Failure::Io(message),
            ) => JobState::Failed { message },
        },
    );
}

fn report(emit: &Emitter, job: &Job, state: JobState) {
    emit(JobEvent {
        id: job.id,
        label: job.label.clone(),
        state,
    });
}

/// Run one job from plan to publish.
fn execute(job: &Job, emit: &Emitter, cancel: &AtomicBool) -> Result<Vec<PathBuf>, Failure> {
    let mut planned = plan(&job.request).map_err(|error| Failure::Plan(error.to_string()))?;

    // The engine's side files — ffmpeg's concat list, LibreOffice's private
    // directory — are removed however this job ends, including cancellation.
    let sidecar = Sidecar::of(&planned.backend);
    sidecar
        .prepare()
        .map_err(|error| Failure::Io(error.to_string()))?;
    // Intermediates of a chain are removed on every exit path, including an
    // engine that fails halfway.
    let _temps = TempFiles::new(planned.temps.clone());

    let duration = media_duration(&planned.backend);

    report(emit, job, JobState::Running { percent: None });

    if let Err(error) = run_backend(
        &planned.backend,
        planned.output.scratch_path(),
        emit,
        job,
        cancel,
        duration,
    ) {
        sweep_series(planned.series.as_ref());
        return Err(error);
    }

    if let Some(series) = &planned.series {
        let produced = series_files(&series.prefix, &series.ext);
        let published = planned
            .output
            .publish_series(&series.tag, &series.ext, &produced)
            .map_err(|error| Failure::Io(error.to_string()))?;
        // The published links are the result; the engine's own names go away.
        for path in &produced {
            let _ = fs::remove_file(path);
        }
        return Ok(published);
    }

    planned
        .output
        .commit()
        .map(|path| vec![path])
        .map_err(|error| Failure::Io(error.to_string()))
}

/// Run one backend, writing its result to `target`.
fn run_backend(
    backend: &Backend,
    target: &Path,
    emit: &Emitter,
    job: &Job,
    cancel: &AtomicBool,
    duration: Option<f64>,
) -> Result<(), Failure> {
    match backend {
        // The built-in engine is a function call, not a child process: it writes
        // the same scratch file and reports the same failure words.
        Backend::Native(op) => {
            if cancel.load(Ordering::Relaxed) {
                return Err(Failure::Cancelled);
            }
            native::run(op, target).map_err(|error| Failure::Native(error.to_string()))
        }
        Backend::Process(spec) => {
            run_process(spec, emit, job, cancel, duration)?;
            collect(spec, target)
        }
        Backend::Chain(steps) => {
            for step in steps {
                run_backend(&step.backend, &step.output, emit, job, cancel, duration)?;
            }
            Ok(())
        }
    }
}

/// The first step of a chain that can report progress, if any.
fn media_duration(backend: &Backend) -> Option<f64> {
    match backend {
        Backend::Process(spec) if spec.progress == ProgressMode::FfmpegKeyValue => {
            spec.media_input.as_deref().and_then(probe_duration)
        }
        Backend::Process(_) | Backend::Native(_) => None,
        Backend::Chain(steps) => steps.iter().find_map(|step| media_duration(&step.backend)),
    }
}

/// Every file the engine wrote for a series, in numeric order.
fn series_files(prefix: &Path, ext: &str) -> Vec<PathBuf> {
    let (Some(dir), Some(base)) = (prefix.parent(), prefix.file_name()) else {
        return Vec::new();
    };
    // A result beside a relative source has an empty parent, which `read_dir`
    // rejects; it means the current directory.
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let base = base.to_string_lossy();
    let suffix = format!(".{}", ext.trim_start_matches('.'));
    let mut found: Vec<(u64, PathBuf)> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let rest = name.strip_prefix(base.as_ref())?.strip_prefix('-')?;
            let number = rest.strip_suffix(&suffix)?.parse::<u64>().ok()?;
            Some((number, entry.path()))
        })
        .collect();
    found.sort_by_key(|(number, _)| *number);
    found.into_iter().map(|(_, path)| path).collect()
}

/// Remove whatever a failed series left behind.
fn sweep_series(series: Option<&crate::plan::SeriesPlan>) {
    let Some(series) = series else {
        return;
    };
    for path in series_files(&series.prefix, &series.ext) {
        let _ = fs::remove_file(path);
    }
}

/// Intermediate chain files, removed when the job ends.
struct TempFiles(Vec<PathBuf>);

impl TempFiles {
    fn new(paths: Vec<PathBuf>) -> Self {
        Self(paths)
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}

/// Spawn the engine and wait for it, reporting progress and honouring cancels.
fn run_process(
    spec: &CommandSpec,
    emit: &Emitter,
    job: &Job,
    cancel: &AtomicBool,
    duration: Option<f64>,
) -> Result<(), Failure> {
    if std::env::var_os("ORBIT_TRACE").is_some() {
        eprintln!(
            "orbit job {}: {} {:?}",
            job.id,
            spec.program.to_string_lossy(),
            spec.args
        );
    }

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // A process group of its own lets a cancel take the engine's children
        // too. Otherwise `child.kill()` stops only the shell or launcher, and a
        // surviving grandchild keeps the stdout pipe open, so the wait blocks
        // for as long as the engine would have run.
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .map_err(|error| Failure::Io(error.to_string()))?;

    let progress = child
        .stdout
        .take()
        .map(|stdout| watch_progress(stdout, Arc::clone(emit), job, duration));

    let stderr_tail = child.stderr.take().map(collect_stderr);

    // Poll rather than block, so cancellation always lands promptly.
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            terminate(&mut child);
            let _ = child.wait();
            if let Some(progress) = progress {
                let _ = progress.join();
            }
            return Err(Failure::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(POLL),
            Err(error) => return Err(Failure::Io(error.to_string())),
        }
    };

    if let Some(progress) = progress {
        let _ = progress.join();
    }
    let stderr = stderr_tail
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();

    if !status.success() {
        let code = status
            .code()
            .map_or_else(|| "signal".to_owned(), |code| code.to_string());
        return Err(Failure::Exit(
            last_line(&stderr).unwrap_or_else(|| format!("engine exited with {code}")),
        ));
    }
    Ok(())
}

/// Stop an engine and everything it started.
///
/// `Child::kill` only signals the process itself. Engines such as `LibreOffice`
/// and `Ghostscript` start helpers of their own, so the whole process group is
/// signalled on Unix; Windows gets the equivalent through `taskkill /T`.
fn terminate(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(child.id()) {
            // A negative pid addresses the process group created at spawn.
            let _ = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(-pid),
                nix::sys::signal::Signal::SIGKILL,
            );
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// Move an engine-named result onto the scratch path.
///
/// `LibreOffice` writes `<stem>.<ext>` where it likes and cannot be told
/// otherwise. Anything else — `pdftoppm`, `tesseract` — appends a known
/// extension to a prefix the planner chose, so it already lands on the scratch
/// path and this does nothing.
fn collect(spec: &CommandSpec, target: &Path) -> Result<(), Failure> {
    let Some(collected) = &spec.collected else {
        return Ok(());
    };

    let wanted = collected.dir.join(&collected.name);
    let source = if wanted.is_file() {
        wanted
    } else {
        // The engine renamed it, or wrote it under a stem of its own choosing:
        // take the file it did write, as long as there is exactly one.
        Sidecar::single_output(&collected.dir).ok_or_else(|| {
            Failure::Exit(format!("the engine wrote nothing named {}", collected.name))
        })?
    };

    fs::rename(&source, target).map_err(|error| Failure::Io(error.to_string()))
}

/// Side paths the engine needs that are not the result.
///
/// Created before the run and removed after it, whatever the outcome: a job
/// that fails must leave the folder exactly as it found it.
struct Sidecar {
    dirs: Vec<PathBuf>,
    aux: Vec<AuxFile>,
}

impl Sidecar {
    fn of(backend: &Backend) -> Self {
        let mut dirs = Vec::new();
        let mut aux = Vec::new();
        sidecar_parts(backend, &mut dirs, &mut aux);
        Self { dirs, aux }
    }

    /// Create the work directories and write the auxiliary files.
    fn prepare(&self) -> io::Result<()> {
        for dir in &self.dirs {
            fs::create_dir_all(dir)?;
        }
        for aux in &self.aux {
            // A relative scratch path has an empty parent, which is the
            // current directory and needs no creating.
            if let Some(parent) = aux
                .path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                fs::create_dir_all(parent)?;
            }
            fs::write(&aux.path, &aux.contents)?;
        }
        Ok(())
    }

    /// The one file an engine left in `dir`, when there is exactly one.
    fn single_output(dir: &Path) -> Option<PathBuf> {
        let mut candidates = fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file());
        let first = candidates.next()?;
        candidates.next().is_none().then_some(first)
    }
}

/// Gather every side file a backend tree needs, without constructing a
/// [`Sidecar`] for each step (whose `Drop` would delete them immediately).
fn sidecar_parts(backend: &Backend, dirs: &mut Vec<PathBuf>, aux: &mut Vec<AuxFile>) {
    match backend {
        Backend::Native(_) => {}
        Backend::Process(spec) => {
            dirs.extend(spec.work_dirs.iter().cloned());
            aux.extend(spec.aux_files.iter().cloned());
        }
        Backend::Chain(steps) => {
            for step in steps {
                sidecar_parts(&step.backend, dirs, aux);
            }
        }
    }
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        for aux in &self.aux {
            let _ = fs::remove_file(&aux.path);
        }
        for dir in &self.dirs {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// Follow ffmpeg's `-progress` stream on its own thread and report movement.
///
/// Returns the handle so the caller can join before publishing, which keeps
/// events from arriving after the job's terminal state.
fn watch_progress(
    stdout: ChildStdout,
    emit: Emitter,
    job: &Job,
    duration: Option<f64>,
) -> JoinHandle<()> {
    let id = job.id;
    let label = job.label.clone();
    thread::spawn(move || {
        let mut last_reported: Option<f64> = None;
        let mut reported_unknown = false;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let Some((key, value)) = split_pair(&line) else {
                continue;
            };
            if key != "out_time_us" && key != "out_time" {
                continue;
            }
            let Some(seconds) = seconds_from(&key, &value) else {
                continue;
            };
            let percent = duration
                .filter(|total| *total > 0.0)
                .map(|total| (seconds / total * 100.0).clamp(0.0, 100.0));

            match percent {
                // Throttle: ffmpeg emits many lines a second and a UI only
                // needs to hear about whole-number movement.
                Some(current) => {
                    let moved_on =
                        last_reported.is_none_or(|previous| (current - previous).abs() >= 1.0);
                    if moved_on {
                        last_reported = Some(current);
                        emit(JobEvent {
                            id,
                            label: label.clone(),
                            state: JobState::Running {
                                percent: Some(current),
                            },
                        });
                    }
                }
                // No duration to measure against: say "running" once rather
                // than once per output line.
                None if !reported_unknown => {
                    reported_unknown = true;
                    emit(JobEvent {
                        id,
                        label: label.clone(),
                        state: JobState::Running { percent: None },
                    });
                }
                None => {}
            }
        }
    })
}

/// Read an engine's stderr into a bounded string, for the failure message.
fn collect_stderr(mut stderr: ChildStderr) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut buffer = Vec::new();
        let _ = stderr.by_ref().take(STDERR_LIMIT).read_to_end(&mut buffer);
        String::from_utf8_lossy(&buffer).trim().to_owned()
    })
}

/// `out_time_us=5000000` and `out_time=00:00:05.000000` both end up here.
fn split_pair(line: &str) -> Option<(String, String)> {
    let (key, value) = line.split_once('=')?;
    Some((key.trim().to_owned(), value.trim().to_owned()))
}

/// Seconds represented by one ffmpeg progress field.
fn seconds_from(key: &str, value: &str) -> Option<f64> {
    if key == "out_time_us" {
        return value.parse::<f64>().ok().map(|micros| micros / 1_000_000.0);
    }
    // `HH:MM:SS.micros`
    let mut parts = value.split(':');
    let hours: f64 = parts.next()?.parse().ok()?;
    let minutes: f64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    Some(hours.mul_add(3600.0, minutes.mul_add(60.0, seconds)))
}

/// Duration in seconds, or `None` when the probe tool is unavailable.
fn probe_duration(input: &Path) -> Option<f64> {
    let output = Command::new(PROBE)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(input)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|seconds| *seconds > 0.0)
}

/// The most useful line of an engine's complaint: its last non-empty one.
fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::plan::StepRequest;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::time::Instant;

    use orbit_registry::Kind;
    use serde_json::json;

    /// A throwaway directory that removes itself.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("orbit-runner-{label}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A pretend engine that *is* the source file, so the runner's real spawn
    /// path is exercised without needing `ffmpeg` or `ImageMagick` installed.
    fn fake_engine(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("fake-engine.sh");
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        path
    }

    /// A job whose "engine" is a shell script. `ImageMagick`'s shape is used
    /// because it is the simplest: inputs first, output last.
    fn script_job(id: JobId, script: &Path, tag: &str) -> Job {
        Job {
            id,
            label: "Convert to WEBP".to_owned(),
            request: JobRequest {
                engine: "magick".to_owned(),
                program: script.to_path_buf(),
                kind: Kind::Image,
                action: "convert".to_owned(),
                target_ext: "webp".to_owned(),
                tag: tag.to_owned(),
                sources: vec![script.to_path_buf()],
                options: json!({})
                    .as_object()
                    .cloned()
                    .unwrap()
                    .into_iter()
                    .collect(),
                output_dir: None,
                steps: Vec::new(),
            },
        }
    }

    /// Run a job to completion and return every event it produced.
    ///
    /// Stops at the terminal state rather than waiting for the channel to close:
    /// the pool holds the emitter — and so the sender — until it is shut down.
    fn run_to_end(job: Job) -> Vec<JobEvent> {
        let (tx, rx) = mpsc::channel();
        let mut runner = Runner::new(1, move |event| {
            let _ = tx.send(event);
        });
        assert!(runner.submit(job));

        let mut events = Vec::new();
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(30)) {
            let finished = !matches!(event.state, JobState::Queued | JobState::Running { .. });
            events.push(event);
            if finished {
                break;
            }
        }
        runner.shutdown();
        events
    }

    fn final_state(events: &[JobEvent]) -> &JobState {
        &events.last().expect("at least one event").state
    }

    #[test]
    fn a_job_reports_queued_running_done_and_publishes_the_file() {
        let dir = TempDir::new("success");
        let script = fake_engine(
            dir.path(),
            "for last; do :; done\nprintf 'real output' > \"$last\"",
        );
        let events = run_to_end(script_job(1, &script, "converted"));

        assert!(matches!(events[0].state, JobState::Queued));
        assert!(matches!(events[1].state, JobState::Running { .. }));
        match final_state(&events) {
            JobState::Done { path } => {
                assert_eq!(path.file_name().unwrap(), "fake-engine (converted).webp");
                assert_eq!(fs::read(path).unwrap(), b"real output");
            }
            other => panic!("expected Done, got {other:?}"),
        }
        // The scratch file is gone and no leftovers remain.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".orbit-"))
            .collect();
        assert!(leftovers.is_empty(), "scratch files are cleaned up");
    }

    #[test]
    fn a_failing_engine_reports_its_own_words_and_leaves_nothing() {
        let dir = TempDir::new("failure");
        let script = fake_engine(dir.path(), "echo 'fake-engine: no such format' >&2\nexit 3");
        let events = run_to_end(script_job(7, &script, "converted"));

        match final_state(&events) {
            JobState::Failed { message } => {
                assert!(message.contains("no such format"), "got: {message}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(
            fs::read_dir(dir.path()).unwrap().count(),
            1,
            "only the script remains"
        );
    }

    #[test]
    fn a_planned_job_that_cannot_run_fails_before_spawning() {
        let dir = TempDir::new("plan-error");
        let script = fake_engine(dir.path(), "echo should not run");
        let mut job = script_job(2, &script, "cropped");
        job.request.action = "crop".to_owned(); // needs a "region"

        let events = run_to_end(job);
        match final_state(&events) {
            JobState::Failed { message } => assert!(message.contains("region"), "got: {message}"),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.state, JobState::Running { .. }))
        );
    }

    #[test]
    fn cancelling_kills_the_child_promptly_and_publishes_nothing() {
        let dir = TempDir::new("cancel");
        let script = fake_engine(dir.path(), "sleep 30");

        let (tx, rx) = mpsc::channel();
        let mut runner = Runner::new(1, move |event| {
            let _ = tx.send(event);
        });
        assert!(runner.submit(script_job(9, &script, "converted")));

        // Wait until it is actually running, then pull the plug.
        let started = Instant::now();
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(5)) {
            if matches!(event.state, JobState::Running { .. }) {
                break;
            }
        }
        runner.cancel(9);
        let last = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("a final event");

        assert!(matches!(last.state, JobState::Cancelled));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancel should not wait for the job to finish"
        );
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            leftovers,
            ["fake-engine.sh"],
            "nothing half-written is left"
        );
        runner.shutdown();
    }

    #[test]
    fn shutdown_abandons_queued_work_instead_of_running_it() {
        let dir = TempDir::new("shutdown");
        let script = fake_engine(dir.path(), "sleep 30");
        let (tx, rx) = mpsc::channel();
        let mut runner = Runner::new(1, move |event| {
            let _ = tx.send(event);
        });
        for id in 1..=4 {
            assert!(runner.submit(script_job(id, &script, "converted")));
        }

        let started = Instant::now();
        runner.shutdown();
        let elapsed = started.elapsed();

        // Four thirty-second jobs. Running the backlog on the way out would
        // take two minutes; quitting has to be immediate.
        assert!(
            elapsed < Duration::from_secs(10),
            "shutdown took {elapsed:?}"
        );
        assert!(
            !runner.submit(script_job(99, &script, "converted")),
            "the pool is closed"
        );

        let events: Vec<JobEvent> = rx.try_iter().collect();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event.state, JobState::Done { .. })),
            "nothing queued should have been run"
        );
    }

    #[test]
    fn progress_fields_are_parsed_in_both_ffmpeg_dialects() {
        assert_eq!(
            split_pair("out_time_us=5000000"),
            Some(("out_time_us".to_owned(), "5000000".to_owned()))
        );
        assert_eq!(seconds_from("out_time_us", "2500000"), Some(2.5));
        assert_eq!(seconds_from("out_time", "00:00:05.500000"), Some(5.5));
        assert_eq!(seconds_from("out_time", "01:02:03.000000"), Some(3723.0));
        assert_eq!(seconds_from("out_time", "not-a-time"), None);
        assert_eq!(
            split_pair("progress=continue"),
            Some(("progress".to_owned(), "continue".to_owned()))
        );
        assert_eq!(split_pair("no equals sign"), None);
    }

    #[test]
    fn the_reported_error_is_the_engines_last_word() {
        assert_eq!(
            last_line("line one\nline two\n\n"),
            Some("line two".to_owned())
        );
        assert_eq!(last_line("   \n\n"), None);
    }

    #[test]
    fn a_zero_sized_pool_still_works() {
        let dir = TempDir::new("zero-workers");
        let script = fake_engine(dir.path(), "for last; do :; done\nprintf ok > \"$last\"");
        let (tx, rx) = mpsc::channel();
        let mut runner = Runner::new(0, move |event| {
            let _ = tx.send(event);
        });
        assert!(runner.submit(script_job(1, &script, "converted")));
        let mut events = Vec::new();
        while let Ok(event) = rx.recv_timeout(Duration::from_secs(30)) {
            let finished = !matches!(event.state, JobState::Queued | JobState::Running { .. });
            events.push(event);
            if finished {
                break;
            }
        }
        assert!(matches!(final_state(&events), JobState::Done { .. }));
        runner.shutdown();
    }

    /// A PDF-render job: `poppler`'s shape is a prefix the engine numbers.
    fn series_job(id: JobId, script: &Path, source: &Path) -> Job {
        Job {
            id,
            label: "Convert to PNG".to_owned(),
            request: JobRequest {
                engine: "poppler".to_owned(),
                program: script.to_path_buf(),
                kind: Kind::Pdf,
                action: "convert".to_owned(),
                target_ext: "png".to_owned(),
                tag: "converted".to_owned(),
                sources: vec![source.to_path_buf()],
                options: json!({ "pages": "1-" })
                    .as_object()
                    .cloned()
                    .unwrap()
                    .into_iter()
                    .collect(),
                output_dir: None,
                steps: Vec::new(),
            },
        }
    }

    #[test]
    fn a_series_is_published_as_a_numbered_run() {
        let dir = TempDir::new("series-run");
        let source = dir.path().join("scan.pdf");
        fs::write(&source, b"%PDF").unwrap();
        let script = fake_engine(
            dir.path(),
            "for last; do :; done\nprintf one > \"${last}-1.png\"\nprintf two > \"${last}-2.png\"",
        );

        let events = run_to_end(series_job(1, &script, &source));
        match final_state(&events) {
            JobState::DoneSeries { paths } => {
                let names: Vec<_> = paths
                    .iter()
                    .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                    .collect();
                assert_eq!(
                    names,
                    ["scan (converted 001).png", "scan (converted 002).png"]
                );
                assert_eq!(fs::read(&paths[0]).unwrap(), b"one");
                assert_eq!(fs::read(&paths[1]).unwrap(), b"two");
            }
            other => panic!("expected DoneSeries, got {other:?}"),
        }
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".orbit-"))
            .collect();
        assert!(leftovers.is_empty(), "scratch series files are cleaned up");
    }

    #[test]
    fn a_chain_runs_every_step_and_cleans_its_intermediates() {
        let dir = TempDir::new("chain-run");
        let source = dir.path().join("photo.png");
        fs::write(&source, b"input").unwrap();
        let counter = dir.path().join("counter");
        let script = fake_engine(
            dir.path(),
            &format!(
                "for last; do :; done\nn=$(cat {counter} 2>/dev/null || echo 0); \
                 n=$((n+1)); echo $n > {counter}; printf \"step$n\" > \"$last\"",
                counter = counter.display()
            ),
        );

        let job = Job {
            id: 3,
            label: "Web ready".to_owned(),
            request: JobRequest {
                engine: "magick".to_owned(),
                program: script,
                kind: Kind::Image,
                action: "chain".to_owned(),
                target_ext: "webp".to_owned(),
                tag: "web-ready".to_owned(),
                sources: vec![source],
                options: json!({})
                    .as_object()
                    .cloned()
                    .unwrap()
                    .into_iter()
                    .collect(),
                output_dir: None,
                steps: vec![
                    StepRequest {
                        engine: "magick".to_owned(),
                        program: PathBuf::from("unused"),
                        kind: Kind::Image,
                        action: "resize".to_owned(),
                        target_ext: "png".to_owned(),
                        tag: "resized".to_owned(),
                        options: json!({})
                            .as_object()
                            .cloned()
                            .unwrap()
                            .into_iter()
                            .collect(),
                    },
                    StepRequest {
                        engine: "magick".to_owned(),
                        program: PathBuf::from("unused"),
                        kind: Kind::Image,
                        action: "convert".to_owned(),
                        target_ext: "webp".to_owned(),
                        tag: "converted".to_owned(),
                        options: json!({})
                            .as_object()
                            .cloned()
                            .unwrap()
                            .into_iter()
                            .collect(),
                    },
                ],
            },
        };

        // `plan_chain` builds each step from the request's own `program`, which
        // it takes from the request, not from the step; make them match.
        let mut job = job;
        for step in &mut job.request.steps {
            step.program = job.request.program.clone();
        }

        let events = run_to_end(job);
        match final_state(&events) {
            JobState::Done { path } => {
                assert_eq!(path.file_name().unwrap(), "photo (web-ready).webp");
                assert_eq!(fs::read(path).unwrap(), b"step2");
            }
            other => panic!("expected Done, got {other:?}"),
        }
        assert_eq!(fs::read(&counter).unwrap(), b"2\n");
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".orbit-"))
            .collect();
        assert!(leftovers.is_empty(), "chain intermediates are removed");
    }
}
