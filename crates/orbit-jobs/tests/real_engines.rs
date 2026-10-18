//! End-to-end runs against the real engines.
//!
//! The unit tests assert the argv Orbit *builds*; these assert what the actual
//! tools do with it, which is the only way to catch a flag ffmpeg rejects, a
//! muxer that dislikes an extension, or a filter with the wrong arity.
//!
//! Each test skips itself (loudly) when its engine is not installed, so the
//! suite stays useful on a machine without them rather than failing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use orbit_jobs::{Job, JobEvent, JobRequest, JobState, Options, Runner};
use orbit_registry::Kind;
use serde_json::json;

/// Long enough that ffmpeg emits several progress reports.
const TONE_SECONDS: &str = "4";

/// Whether a tool answers on `PATH`.
fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// A throwaway directory that removes itself.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("orbit-e2e-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
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

/// A real PNG, made by the tool itself so no binary fixture lives in the repo.
fn make_png(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("magick")
        .args(["-size", "64x48", "gradient:red-blue"])
        .arg(&path)
        .status()
        .expect("run magick to build a fixture");
    assert!(status.success(), "could not build the PNG fixture");
    path
}

/// A real WAV tone, for the same reason.
fn make_wav(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let status = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=440:duration={TONE_SECONDS}"),
        ])
        .arg(&path)
        .status()
        .expect("run ffmpeg to build a fixture");
    assert!(status.success(), "could not build the WAV fixture");
    path
}

/// A shell script that prints ffmpeg progress lines and then "encodes".
///
/// Pairing it with a *real* WAV means the runner's own `ffprobe` duration probe
/// runs for real, so the percentages below are the ones a user would see.
fn progress_stub(dir: &Path) -> PathBuf {
    let path = dir.join("stub-engine.sh");
    fs::write(
        &path,
        "#!/bin/sh\n\
         printf 'out_time_us=1000000\\n'\n\
         printf 'progress=continue\\n'\n\
         sleep 0.4\n\
         printf 'out_time_us=2000000\\n'\n\
         for last; do :; done\n\
         printf 'encoded' > \"$last\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }
    path
}

/// Run one job and collect every event, stopping at the terminal state.
fn run(request: JobRequest) -> Vec<JobEvent> {
    let (tx, rx) = mpsc::channel();
    let mut runner = Runner::new(1, move |event| {
        let _ = tx.send(event);
    });
    assert!(
        runner.submit(Job {
            id: 1,
            label: "Convert".to_owned(),
            request,
        }),
        "the pool accepted the job"
    );

    let mut events = Vec::new();
    while let Ok(event) = rx.recv_timeout(Duration::from_secs(120)) {
        let finished = !matches!(event.state, JobState::Queued | JobState::Running { .. });
        events.push(event);
        if finished {
            break;
        }
    }
    runner.shutdown();
    events
}

fn done_path(events: &[JobEvent]) -> PathBuf {
    match &events.last().expect("at least one event").state {
        JobState::Done { path } => path.clone(),
        other => panic!("expected a finished job, got {other:?}"),
    }
}

/// Options from a JSON literal, the way the shell would hand them over.
fn options(value: serde_json::Value) -> Options {
    match value {
        serde_json::Value::Object(map) => map.into_iter().collect(),
        _ => Options::new(),
    }
}

fn image_request(source: &Path, target_ext: &str, opts: serde_json::Value) -> JobRequest {
    JobRequest {
        engine: "magick".to_owned(),
        program: PathBuf::from("magick"),
        kind: Kind::Image,
        action: "convert".to_owned(),
        target_ext: target_ext.to_owned(),
        tag: "converted".to_owned(),
        sources: vec![source.to_path_buf()],
        options: options(opts),
        output_dir: None,
    }
}

#[test]
fn magick_really_converts_a_png_to_webp() {
    if !have("magick") {
        eprintln!("skipping: ImageMagick is not installed");
        return;
    }
    let dir = TempDir::new("magick-webp");
    let source = make_png(dir.path(), "holiday.png");

    let events = run(image_request(&source, "webp", json!({ "quality": 80 })));
    let published = done_path(&events);

    assert_eq!(
        published.file_name().unwrap(),
        "holiday (converted).webp",
        "the documented naming rule"
    );
    let bytes = fs::read(&published).unwrap();
    assert!(bytes.len() > 12, "a real image came out");
    // RIFF....WEBP
    assert_eq!(&bytes[0..4], b"RIFF");
    assert_eq!(&bytes[8..12], b"WEBP");

    // The source is untouched and no scratch file survived.
    assert!(source.exists());
    let leftovers: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".orbit-"))
        .collect();
    assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
}

#[test]
fn ffmpeg_really_converts_a_wav_to_mp3_without_overwriting() {
    if !have("ffmpeg") {
        eprintln!("skipping: ffmpeg is not installed");
        return;
    }
    let dir = TempDir::new("ffmpeg-mp3");
    let source = make_wav(dir.path(), "tone.wav");

    let request = JobRequest {
        engine: "ffmpeg".to_owned(),
        program: PathBuf::from("ffmpeg"),
        kind: Kind::Audio,
        action: "convert".to_owned(),
        target_ext: "mp3".to_owned(),
        tag: "converted".to_owned(),
        sources: vec![source.clone()],
        options: options(json!({ "audio_bitrate": "192000" })),
        output_dir: None,
    };

    let first = done_path(&run(request.clone()));
    assert_eq!(first.file_name().unwrap(), "tone (converted).mp3");
    let bytes = fs::read(&first).unwrap();
    assert!(bytes.len() > 1000, "a real audio file came out");
    // Either an ID3 tag or a bare MPEG frame sync.
    let looks_like_mp3 =
        bytes.starts_with(b"ID3") || (bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0);
    assert!(looks_like_mp3, "the output is not MP3");

    // Running it again must add a file, never replace the first.
    let second = done_path(&run(request));
    assert_eq!(second.file_name().unwrap(), "tone (converted 2).mp3");
    assert_eq!(
        fs::read(&first).unwrap(),
        bytes,
        "the first result is intact"
    );
}

#[test]
fn conversion_progress_is_reported_as_percentages() {
    if !have("ffmpeg") {
        eprintln!("skipping: ffmpeg is not installed (needed for the duration probe)");
        return;
    }
    let dir = TempDir::new("progress");
    let source = make_wav(dir.path(), "tone.wav");

    let request = JobRequest {
        engine: "ffmpeg".to_owned(),
        program: progress_stub(dir.path()),
        kind: Kind::Audio,
        action: "convert".to_owned(),
        target_ext: "mp3".to_owned(),
        tag: "converted".to_owned(),
        sources: vec![source.clone()],
        options: options(json!({})),
        output_dir: None,
    };

    let events = run(request);
    assert!(
        matches!(events.last().unwrap().state, JobState::Done { .. }),
        "the stub should have finished: {:?}",
        events.last()
    );

    let percents: Vec<f64> = events
        .iter()
        .filter_map(|event| match event.state {
            JobState::Running {
                percent: Some(percent),
            } => Some(percent),
            _ => None,
        })
        .collect();

    // The stub reports 1s and 2s of a four-second file.
    assert!(
        percents.contains(&25.0) && percents.contains(&50.0),
        "expected 25% and 50% among {percents:?}"
    );
    assert!(
        percents.windows(2).all(|pair| pair[1] >= pair[0]),
        "progress must never go backwards: {percents:?}"
    );
    assert!(
        percents
            .iter()
            .all(|percent| (0.0..=100.0).contains(percent))
    );
}

#[test]
fn a_missing_engine_fails_with_a_readable_reason() {
    // No skip here: this must hold on every machine.
    let dir = TempDir::new("no-engine");
    // The content is irrelevant: this request fails while planning, before any
    // engine is started, so no fixture tooling is needed.
    let source = dir.path().join("holiday.png");
    fs::write(&source, b"not really a png").unwrap();

    let events = run(JobRequest {
        engine: "ghostscript".to_owned(), // not driven yet
        program: PathBuf::from("gs"),
        kind: Kind::Image,
        action: "convert".to_owned(),
        target_ext: "pdf".to_owned(),
        tag: "converted".to_owned(),
        sources: vec![source],
        options: options(json!({})),
        output_dir: None,
    });

    match &events.last().unwrap().state {
        JobState::Failed { message } => {
            assert!(
                message.contains("M3"),
                "should name the milestone: {message}"
            );
        }
        other => panic!("expected Failed, got {other:?}"),
    }
    // Nothing was written, not even a scratch file.
    let entries: Vec<String> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, ["holiday.png"], "only the source remains");
}
