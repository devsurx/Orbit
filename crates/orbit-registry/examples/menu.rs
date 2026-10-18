//! Prints the radial menu for one or more extensions.
//!
//! Development tool, and the fastest way to answer "what does a .mov actually
//! offer, and in what order?":
//!
//! ```text
//! cargo run -p orbit-registry --example menu -- mov png heic
//! cargo run -p orbit-registry --example menu -- --available ffmpeg,magick mov
//! cargo run -p orbit-registry --example menu -- --pairs
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::process::ExitCode;

use orbit_registry::{MenuRequest, Registry, SourceFile};

fn main() -> ExitCode {
    let registry = match Registry::bundled() {
        Ok(registry) => registry,
        Err(error) => {
            eprintln!("registry is not usable:\n{error}");
            return ExitCode::FAILURE;
        }
    };

    let mut available: Option<BTreeSet<String>> = None;
    let mut show_pairs = false;
    let mut extensions = Vec::new();

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--available" => {
                let list = args.next().unwrap_or_default();
                available = Some(
                    list.split(',')
                        .map(str::trim)
                        .filter(|item| !item.is_empty())
                        .map(str::to_owned)
                        .collect(),
                );
            }
            "--pairs" => show_pairs = true,
            other => extensions.push(other.trim_start_matches('.').to_ascii_lowercase()),
        }
    }

    if show_pairs {
        let set = available.unwrap_or_else(|| registry.all_engine_ids());
        print_pairs(&registry, &set);
        return ExitCode::SUCCESS;
    }

    if extensions.is_empty() {
        eprintln!("usage: menu [--available a,b,c] [--pairs] <ext> [ext...]");
        return ExitCode::FAILURE;
    }

    let files: Vec<SourceFile> = extensions
        .iter()
        .map(|ext| SourceFile::virtual_file(registry.probe_ext(ext)))
        .collect();

    let mut request = MenuRequest::for_files(files);
    request.available.clone_from(&available);
    let menu = registry.menu(&request);

    println!(
        "files: {}   candidates: {}   shared: {:?}",
        menu.files
            .iter()
            .map(|file| file.name.clone())
            .collect::<Vec<_>>()
            .join(", "),
        menu.candidates,
        menu.shared_kind
    );
    if let Some(set) = &available {
        let missing: Vec<&str> = registry
            .engines()
            .iter()
            .filter(|engine| !set.contains(&engine.id))
            .map(|engine| engine.id.as_str())
            .collect();
        println!("assuming installed: {set:?}");
        println!("treated as absent: {}", missing.join(", "));
    } else {
        println!("assuming every engine is installed");
    }

    for category in &menu.categories {
        println!(
            "\n{} [{}]{}",
            category.label,
            category.id,
            if category.hidden > 0 {
                format!(" (+{} hidden)", category.hidden)
            } else {
                String::new()
            }
        );
        for petal in &category.petals {
            let state = match &petal.disabled {
                Some(disabled) => format!("  DISABLED: {}", disabled.reason),
                None => String::new(),
            };
            println!(
                "  {:<20} {:<10} score {:<6} cover {}/{}{}{}",
                petal.id,
                format!("{:?}", petal.kind).to_lowercase(),
                petal.score,
                petal.coverage,
                petal.total,
                petal
                    .badge
                    .as_ref()
                    .map_or(String::new(), |badge| format!("  [{badge}]")),
                state
            );
        }
    }

    ExitCode::SUCCESS
}

/// Print the reachable `from -> to` matrix for the engines assumed installed.
fn print_pairs(registry: &Registry, set: &BTreeSet<String>) {
    let pairs = registry.usable_pairs(set);
    let mut by_family: BTreeMap<String, usize> = BTreeMap::new();
    for pair in &pairs {
        *by_family
            .entry(format!("{} -> {}", pair.from_kind, pair.to_kind))
            .or_default() += 1;
    }
    println!("{:<24} {}", "pairs", pairs.len());
    println!("{:<24} {}", "engines installed", set.len());
    for (family, count) in by_family {
        println!("  {family:<30} {count}");
    }
    println!("\ntop 25 by score:");
    for pair in pairs.iter().take(25) {
        println!(
            "  {:<8} -> {:<8} rank {:<5} engine {}",
            pair.from,
            pair.to,
            pair.score,
            pair.choice.chosen.as_deref().unwrap_or("none")
        );
    }
}
