//! Orbit's job engine: what to run, where the result goes, and how it gets there.
//!
//! Three concerns, deliberately separated:
//!
//! | Module | Question it answers |
//! |---|---|
//! | [`plan`] | *what command line does this request mean?* |
//! | [`output`] | *which file does it write, and can it clobber anything?* |
//! | [`runner`] | *how does it run without blocking the UI, and stop when asked?* |
//!
//! The split exists so the parts that are easy to get wrong are easy to test.
//! Command building is pure, so every engine invocation can be asserted on
//! without spawning anything; the output policy is exercised against a real
//! filesystem; and the pool is tested with a stand-in "engine" script so the
//! suite does not depend on `ffmpeg` or `ImageMagick` being installed.
//!
//! Nothing here is Orbit-aware beyond the registry's vocabulary: the caller
//! resolves which engine, which extension and which options apply, and hands
//! over a [`JobRequest`]. That keeps this crate reusable by the CLI in M5.

#![forbid(unsafe_code)]

pub mod output;
pub mod plan;
pub mod runner;

pub use output::OutputPlan;
pub use plan::{CommandSpec, JobRequest, Options, Plan, PlanError, ProgressMode, plan};
pub use runner::{Job, JobEvent, JobId, JobState, Runner};
