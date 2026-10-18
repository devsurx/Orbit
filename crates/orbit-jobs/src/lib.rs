//! Orbit's job engine: what to run, where the result goes, and how it gets there.
//!
//! Four concerns, deliberately separated:
//!
//! | Module | Question it answers |
//! |---|---|
//! | [`resolve`] | *what does this pick in the ring actually mean?* |
//! | [`plan`] | *what command line does this request mean?* |
//! | [`output`] | *which file does it write, and can it clobber anything?* |
//! | [`runner`] | *how does it run without blocking the UI, and stop when asked?* |
//!
//! The split exists so the parts that are easy to get wrong are easy to test.
//! Resolution is a pure function of the registry and the dragged paths, command
//! building is pure, the output policy is exercised against a real filesystem,
//! and the pool is tested with a stand-in "engine" script so the suite does not
//! depend on `ffmpeg` or `ImageMagick` being installed.
//!
//! Everything from [`plan`] down is registry-aware only through
//! [`JobRequest`]: the caller resolves which engine, which extension and which
//! options apply, and hands one over. [`resolve`] is the standard way to do that
//! against the bundled registry, which keeps this crate usable by the CLI in M5.

#![forbid(unsafe_code)]

pub mod output;
pub mod plan;
pub mod resolve;
pub mod runner;

pub use output::OutputPlan;
pub use plan::{CommandSpec, JobRequest, Options, Plan, PlanError, ProgressMode, plan};
pub use resolve::{Pick, PickKind, Resolution, ResolveError, Skipped, resolve_pick};
pub use runner::{Job, JobEvent, JobId, JobState, Runner};
