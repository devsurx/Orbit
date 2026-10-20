//! `orbit convert` without the ring.
//!
//! The desktop shell and this binary share every decision: the registry picks
//! the engine, `orbit-jobs` resolves the pick and runs the job, and both write
//! the same history file. The only thing that differs is how the request
//! arrives — a terminal instead of a drag.
//!
//! ```text
//! orbit-cli convert --to webp photo.png
//! orbit-cli action compress report.pdf --option compress_mode=target_size \
//!     --option target_size=5000000
//! orbit-cli preset web-ready holiday.png
//! orbit-cli list formats
//! orbit-cli history --limit 5
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc;
use std::time::Duration;

use orbit_jobs::{
    HistoryEntry, HistoryStore, Job, JobEvent, JobState, Options, Pick, PickKind, Runner,
    resolve_pick,
};
use orbit_registry::{EngineStatus, PetalKind, Registry, host_platform};
use serde_json::Value;

/// How long a terminal job may take before the CLI gives up waiting, in seconds.
const WAIT: Duration = Duration::from_secs(24 * 60 * 60);

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("orbit: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<ExitCode, String> {
    let Some(command) = args.first().map(String::as_str) else {
        print_help();
        return Ok(ExitCode::SUCCESS);
    };

    match command {
        "help" | "--help" | "-h" => {
            print_help();
            Ok(ExitCode::SUCCESS)
        }
        "version" | "--version" | "-V" => {
            println!("orbit {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        "list" | "ls" => list(args.get(1).map(String::as_str)),
        "history" => history(&args[1..]),
        "convert" => convert(&args[1..]),
        "action" => action(&args[1..]),
        "preset" => preset(&args[1..]),
        other => Err(format!(
            "unknown command \"{other}\" — try `orbit-cli help`"
        )),
    }
}

fn print_help() {
    println!(
        r"Orbit — convert files without opening the ring.

USAGE
  orbit-cli convert --to <format> <file>... [--out <dir>] [--option key=value]...
  orbit-cli action <action-id> <file>...  [--out <dir>] [--option key=value]...
  orbit-cli preset <preset-id> <file>...  [--out <dir>]
  orbit-cli list [formats|actions|presets|engines]
  orbit-cli history [--limit N] [--clear]

EXAMPLES
  orbit-cli convert --to webp photo.png
  orbit-cli convert --to png scan.pdf --option pages=1-3
  orbit-cli action extract_frames clip.mp4 --option frame_step=5
  orbit-cli preset web-ready *.jpg --out ~/Pictures/web

Options are the same keys the ring's panel shows (`orbit-cli list actions`).
Every result is written beside its source unless --out is given, and an
existing file is never overwritten."
    );
}

// ------------------------------------------------------------------ commands

fn list(what: Option<&str>) -> Result<ExitCode, String> {
    let registry = Registry::bundled().map_err(|error| error.to_string())?;
    match what.unwrap_or("all") {
        "formats" => {
            for format in registry.formats() {
                let access = match (format.can.read, format.can.write) {
                    (true, true) => "read/write",
                    (true, false) => "read",
                    (false, true) => "write",
                    (false, false) => "metadata",
                };
                println!(
                    "{:<12} {:<10} {:<12} {}",
                    format.id,
                    format.kind,
                    access,
                    format.extensions.join(", ")
                );
            }
        }
        "actions" => {
            for action in registry.actions() {
                println!("{:<18} {}", action.id, action.label);
            }
            for chain in registry.chains() {
                println!("{:<18} {} (preset)", chain.id, chain.label);
            }
        }
        "presets" => {
            for chain in registry.chains() {
                println!("{:<18} {}", chain.id, chain.label);
            }
        }
        "engines" => {
            let statuses = detect(&registry);
            for engine in registry.engines() {
                let status = statuses.get(&engine.id);
                let available = status.is_some_and(|status| status.available);
                let binary = status
                    .and_then(|status| status.binary.as_ref())
                    .map_or_else(|| "-".to_owned(), |path| path.display().to_string());
                println!(
                    "{:<14} {:<8} {}",
                    engine.id,
                    if available { "ready" } else { "missing" },
                    binary
                );
            }
        }
        "all" | "summary" => {
            println!(
                "{} formats · {} actions · {} presets · {} engines",
                registry.formats().len(),
                registry.actions().len(),
                registry.chains().len(),
                registry.engines().len()
            );
            println!("`orbit-cli list formats` for the catalogue");
        }
        other => return Err(format!("unknown list \"{other}\"")),
    }
    Ok(ExitCode::SUCCESS)
}

fn history(args: &[String]) -> Result<ExitCode, String> {
    let store = HistoryStore::default_path();
    if args.iter().any(|arg| arg == "--clear") {
        HistoryStore::new(&store)
            .clear()
            .map_err(|error| format!("could not clear {}: {error}", store.display()))?;
        println!("history cleared");
        return Ok(ExitCode::SUCCESS);
    }
    let limit = args
        .iter()
        .position(|arg| arg == "--limit")
        .and_then(|index| args.get(index + 1))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20);
    let entries = HistoryStore::new(&store).load(limit);
    if entries.is_empty() {
        println!("no history yet ({})", store.display());
        return Ok(ExitCode::SUCCESS);
    }
    for entry in entries {
        let mark = if entry.ok { " " } else { "!" };
        let when = format_time(entry.at);
        let what = entry
            .outputs
            .first()
            .or_else(|| entry.sources.first())
            .map_or_else(String::new, |path| path.display().to_string());
        println!("{mark} {when}  {:<22} {what}", entry.label);
    }
    Ok(ExitCode::SUCCESS)
}

fn convert(args: &[String]) -> Result<ExitCode, String> {
    let (flags, files) = split_flags(args);
    let to = flags
        .get("to")
        .cloned()
        .ok_or_else(|| "convert needs --to <format>".to_owned())?;
    let pick = Pick {
        kind: PickKind::Conversion,
        reference: to,
        defaults: Options::default(),
    };
    execute(pick, files, &flags)
}

fn action(args: &[String]) -> Result<ExitCode, String> {
    let (flags, mut positional) = split_flags(args);
    if positional.is_empty() {
        return Err("action needs an action id".to_owned());
    }
    let reference = positional.remove(0).to_string_lossy().into_owned();
    let pick = Pick {
        kind: PickKind::Action,
        reference,
        defaults: Options::default(),
    };
    execute(pick, positional, &flags)
}

fn preset(args: &[String]) -> Result<ExitCode, String> {
    let (flags, mut positional) = split_flags(args);
    if positional.is_empty() {
        return Err("preset needs a preset id".to_owned());
    }
    let reference = positional.remove(0).to_string_lossy().into_owned();
    let pick = Pick {
        kind: PickKind::Preset,
        reference,
        defaults: Options::default(),
    };
    execute(pick, positional, &flags)
}

// ------------------------------------------------------------------ execution

#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
fn execute(
    mut pick: Pick,
    files: Vec<PathBuf>,
    flags: &BTreeMap<String, String>,
) -> Result<ExitCode, String> {
    if files.is_empty() {
        return Err("no files given".to_owned());
    }
    for file in &files {
        if !file.exists() {
            return Err(format!("{} does not exist", file.display()));
        }
    }

    let registry = Registry::bundled().map_err(|error| error.to_string())?;
    let statuses = detect(&registry);
    pick.defaults = options(flags, &registry, &pick, &files, &statuses);

    let output_dir = flags.get("out").map(PathBuf::from);
    let mut resolution =
        resolve_pick(&registry, &statuses, &pick, &files).map_err(|error| error.to_string())?;
    for request in &mut resolution.requests {
        request.output_dir.clone_from(&output_dir);
    }
    if resolution.is_empty() {
        let reason = resolution
            .first_reason()
            .unwrap_or("no file was suitable")
            .to_owned();
        return Err(reason);
    }
    for skipped in &resolution.skipped {
        eprintln!(
            "orbit: skipped {} — {}",
            skipped.path.display(),
            skipped.reason
        );
    }

    let label = match pick.kind {
        PickKind::Conversion => format!("Convert to {}", pick.reference.to_uppercase()),
        PickKind::Preset => registry
            .chain(&pick.reference)
            .map_or_else(|| pick.reference.clone(), |chain| chain.label.clone()),
        _ => registry
            .action(&pick.reference)
            .map_or_else(|| pick.reference.clone(), |action| action.label.clone()),
    };
    let total = resolution.requests.len();
    let (sender, receiver) = mpsc::channel();
    let mut runner = Runner::new(worker_count(), move |event| {
        let _ = sender.send(event);
    });

    let mut meta: BTreeMap<u64, (String, Vec<PathBuf>)> = BTreeMap::new();
    for (index, request) in resolution.requests.into_iter().enumerate() {
        let id = u64::try_from(index).unwrap_or(0);
        meta.insert(id, (label.clone(), request.sources.clone()));
        if !runner.submit(Job {
            id,
            label: label.clone(),
            request,
        }) {
            return Err("the worker pool stopped".to_owned());
        }
    }

    let mut finished = 0;
    let mut failed = 0;
    let mut outputs: Vec<PathBuf> = Vec::new();
    let mut json: Vec<Value> = Vec::new();
    let store = HistoryStore::new(HistoryStore::default_path());

    while finished < total {
        let Ok(event) = receiver.recv_timeout(WAIT) else {
            return Err("timed out waiting for a job".to_owned());
        };
        match &event.state {
            JobState::Queued | JobState::Running { .. } => continue,
            JobState::Done { path } => {
                finished += 1;
                outputs.push(path.clone());
                if !json_mode(flags) {
                    println!("{}", path.display());
                }
                record(&store, &meta, &event, vec![path.clone()], true, None);
            }
            JobState::DoneSeries { paths } => {
                finished += 1;
                outputs.extend(paths.iter().cloned());
                if !json_mode(flags) {
                    for path in paths {
                        println!("{}", path.display());
                    }
                }
                record(&store, &meta, &event, paths.clone(), true, None);
            }
            JobState::Failed { message } => {
                finished += 1;
                failed += 1;
                eprintln!("orbit: {} — {message}", event.label);
                record(
                    &store,
                    &meta,
                    &event,
                    Vec::new(),
                    false,
                    Some(message.clone()),
                );
            }
            JobState::Cancelled => {
                finished += 1;
                failed += 1;
                eprintln!("orbit: {} cancelled", event.label);
                record(
                    &store,
                    &meta,
                    &event,
                    Vec::new(),
                    false,
                    Some("cancelled".to_owned()),
                );
            }
        }
        json.push(serde_json::json!({
            "id": event.id,
            "label": event.label,
            "state": event.state,
        }));
    }
    runner.shutdown();

    if json_mode(flags) {
        println!(
            "{}",
            serde_json::to_string_pretty(&json).map_err(|error| error.to_string())?
        );
    } else if total > 1 {
        eprintln!(
            "orbit: {total} job(s), {} produced {} file(s)",
            total - failed,
            outputs.len()
        );
    }

    Ok(if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn record(
    store: &HistoryStore,
    meta: &BTreeMap<u64, (String, Vec<PathBuf>)>,
    event: &JobEvent,
    outputs: Vec<PathBuf>,
    ok: bool,
    message: Option<String>,
) {
    let (label, sources) = meta.get(&event.id).cloned().unwrap_or_default();
    let entry = HistoryEntry::now(label, event.label.clone(), sources, outputs, ok, message);
    if let Err(error) = store.append(&entry) {
        eprintln!("orbit: could not record history ({error})");
    }
}

// --------------------------------------------------------------------- flags

/// Split `--flag value`, `--flag=value` and bare files.
fn split_flags(args: &[String]) -> (BTreeMap<String, String>, Vec<PathBuf>) {
    let mut flags = BTreeMap::new();
    let mut files = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if let Some(rest) = arg.strip_prefix("--") {
            let (key, inline) = rest
                .split_once('=')
                .map_or((rest, None), |(key, value)| (key, Some(value)));
            let value = inline.map_or_else(
                || {
                    index += 1;
                    args.get(index).cloned().unwrap_or_default()
                },
                str::to_owned,
            );
            flags.insert(key.to_owned(), value);
        } else if arg.starts_with('-') {
            // Unknown switches are ignored rather than mistaken for files.
        } else {
            files.push(PathBuf::from(arg));
        }
        index += 1;
    }
    (flags, files)
}

fn options(
    flags: &BTreeMap<String, String>,
    registry: &Registry,
    pick: &Pick,
    files: &[PathBuf],
    statuses: &BTreeMap<String, EngineStatus>,
) -> Options {
    // Start from what the ring would have used for a plain drop, then let
    // `--option` override it. That keeps defaults like `pages=1-` and
    // `frame_limit=100` consistent between the two front ends.
    let mut values = ring_defaults(registry, pick, files, statuses);
    for (key, text) in flags {
        if key == "out" || key == "to" || key == "json" {
            continue;
        }
        // `--option` is sugar for `--key value`, which is what the examples use.
        if key == "option" {
            if let Some((option, value)) = text.split_once('=') {
                values.insert(option.to_owned(), Value::String(value.to_owned()));
            }
            continue;
        }
        values.insert(key.clone(), Value::String(text.clone()));
    }
    values
}

fn ring_defaults(
    registry: &Registry,
    pick: &Pick,
    files: &[PathBuf],
    statuses: &BTreeMap<String, EngineStatus>,
) -> Options {
    let mut request = orbit_registry::MenuRequest::for_files(
        files
            .iter()
            .map(|path| {
                let size = std::fs::metadata(path).map_or(0, |meta| meta.len());
                registry.classify(path, size)
            })
            .collect(),
    );
    request.available = Some(orbit_registry::available_ids(statuses));
    let model = registry.menu(&request);
    model
        .categories
        .iter()
        .flat_map(|category| category.petals.iter())
        .find(|petal| petal.reference == pick.reference && same_kind(petal.kind, pick.kind))
        .map(|petal| petal.defaults.clone())
        .unwrap_or_default()
}

/// Whether a petal is the pick the caller asked for.
fn same_kind(petal: PetalKind, pick: PickKind) -> bool {
    matches!(
        (petal, pick),
        (PetalKind::Conversion, PickKind::Conversion)
            | (PetalKind::Action, PickKind::Action)
            | (PetalKind::Preset, PickKind::Preset)
            | (PetalKind::Operator, PickKind::Operator)
    )
}

fn json_mode(flags: &BTreeMap<String, String>) -> bool {
    flags.contains_key("json")
}

fn detect(registry: &Registry) -> BTreeMap<String, EngineStatus> {
    registry.detect_engines(host_platform(), std::env::var("PATH").ok().as_deref())
}

fn worker_count() -> usize {
    std::thread::available_parallelism().map_or(2, |cores| (cores.get() / 2).clamp(1, 4))
}

/// `2026-10-20 14:32` in UTC, without a date-library dependency.
fn format_time(millis: u64) -> String {
    let seconds = millis / 1000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(0);
    let rem = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Howard Hinnant's `civil_from_days`, the inverse of days-since-epoch.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 { y + 1 } else { y },
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_and_files_are_separated() {
        let args: Vec<String> = ["--to", "webp", "a.png", "b.png", "--json"]
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect();
        let (flags, files) = split_flags(&args);
        assert_eq!(flags.get("to").map(String::as_str), Some("webp"));
        assert!(flags.contains_key("json"));
        assert_eq!(files, [PathBuf::from("a.png"), PathBuf::from("b.png")]);
    }

    #[test]
    fn inline_values_are_accepted() {
        let args: Vec<String> = ["--to=webp", "a.png"]
            .iter()
            .map(|arg| (*arg).to_owned())
            .collect();
        let (flags, files) = split_flags(&args);
        assert_eq!(flags.get("to").map(String::as_str), Some("webp"));
        assert_eq!(files, [PathBuf::from("a.png")]);
    }

    #[test]
    fn dates_are_formatted_in_utc() {
        assert_eq!(format_time(0), "1970-01-01 00:00");
        // 2026-10-20 00:00:00 UTC
        assert_eq!(format_time(1_792_454_400_000), "2026-10-20 00:00");
    }

    #[test]
    fn a_missing_command_prints_help_instead_of_panicking() {
        assert!(run(&[]).is_ok());
    }
}
