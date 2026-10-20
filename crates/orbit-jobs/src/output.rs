//! Output policy: where a result lands, and how it gets there intact.
//!
//! Two promises are kept here, and they are the ones users notice when they are
//! broken:
//!
//! * **Nothing is ever overwritten.** A new name is picked, never an existing
//!   file, and publishing is a link rather than a rename so a file that appears
//!   in the meantime cannot be clobbered either.
//! * **A failed job leaves nothing behind.** The engine writes to a hidden
//!   scratch file beside the destination; the plan publishes it on success and
//!   deletes it on drop, so a crash or a cancel cannot litter the folder with
//!   half-written files.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes scratch files created by this process.
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// How many numbered variants to try before giving up on a name.
const MAX_VARIANTS: u32 = 999;

/// A reserved destination plus the scratch file the engine writes first.
///
/// See the module docs for the guarantees. Call [`OutputPlan::commit`] to
/// publish the result; dropping without committing cleans up the scratch file.
#[derive(Debug)]
pub struct OutputPlan {
    /// The source's stem, kept so a *series* can name every file it publishes
    /// (`photo (converted 001).png`) without re-deriving it from `final_path`,
    /// where the tag has already been folded in.
    stem: String,
    final_path: PathBuf,
    scratch_path: PathBuf,
    committed: bool,
}

impl OutputPlan {
    /// Reserve a free destination for `source` and create the scratch file.
    ///
    /// The destination is `"<stem> (<tag>).<ext>"` next to the source, or inside
    /// `folder` when one is given. When that name is taken the variant counter
    /// climbs: `"<stem> (<tag> 2).<ext>"`, then `3`, and so on.
    ///
    /// # Errors
    ///
    /// Fails when the source has no file name to build on, when the destination
    /// folder cannot be created, or when the scratch file cannot be created.
    pub fn reserve(source: &Path, tag: &str, ext: &str, folder: Option<&Path>) -> io::Result<Self> {
        let stem = source
            .file_stem()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "source has no file name"))?
            .to_string_lossy()
            .into_owned();

        let dir = match folder {
            Some(dir) => {
                fs::create_dir_all(dir)?;
                dir.to_path_buf()
            }
            None => source
                .parent()
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf),
        };

        let ext = ext.trim_start_matches('.').to_lowercase();
        let final_path = (1..=MAX_VARIANTS)
            .map(|n| dir.join(variant_name(&stem, tag, &ext, n)))
            .find(|candidate| !candidate.exists())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("every name up to \"{stem} ({tag} {MAX_VARIANTS}).{ext}\" is taken"),
                )
            })?;

        let scratch_path = create_scratch(&dir, &ext)?;
        Ok(Self {
            stem,
            final_path,
            scratch_path,
            committed: false,
        })
    }

    /// The name the result will be published under.
    #[must_use]
    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    /// The source's stem, for naming a series.
    #[must_use]
    pub fn stem(&self) -> &str {
        &self.stem
    }

    /// The folder every result of this job belongs in.
    #[must_use]
    pub fn folder(&self) -> &Path {
        self.final_path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// The scratch path to hand to the engine, which may overwrite it freely.
    #[must_use]
    pub fn scratch_path(&self) -> &Path {
        &self.scratch_path
    }

    /// A hidden path beside the scratch file, for the extra files some engines
    /// need: ffmpeg's concat list, `LibreOffice`'s private directory.
    ///
    /// Named after the scratch file rather than the destination, so it inherits
    /// this job's process id and counter. Two jobs writing into one folder can
    /// therefore never collide, which matters because the scratch counter is
    /// per process and two Orbit windows can share a folder.
    #[must_use]
    pub fn scratch_sibling(&self, label: &str) -> PathBuf {
        scratch_sibling(&self.scratch_path, label)
    }

    /// Publish a run of files as `stem (tag 001).ext`, `stem (tag 002).ext`, …
    ///
    /// The whole run is reserved before anything is linked, so a collision
    /// moves the batch to the next free block instead of leaving a half-numbered
    /// set behind. Each publish is still a hard link, so a file that appears in
    /// the meantime cannot be overwritten.
    ///
    /// # Errors
    ///
    /// Fails when the engine produced nothing, when no free block of names can
    /// be found, or when a link fails. Links already made are removed on
    /// failure, so a failed series publishes either all of it or none.
    pub fn publish_series(
        &mut self,
        tag: &str,
        ext: &str,
        produced: &[PathBuf],
    ) -> io::Result<Vec<PathBuf>> {
        if produced.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the engine wrote no files",
            ));
        }
        let ext = ext.trim_start_matches('.').to_lowercase();
        let count = u32::try_from(produced.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too many files"))?;
        let dir = self.folder().to_path_buf();

        let start = (1..=MAX_VARIANTS)
            .find(|start| {
                (0..count).all(|offset| {
                    start
                        .checked_add(offset)
                        .is_none_or(|index| index <= MAX_VARIANTS)
                        && !dir
                            .join(series_name(&self.stem, tag, start + offset, &ext))
                            .exists()
                })
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("every series name up to {MAX_VARIANTS} is taken"),
                )
            })?;

        let mut published = Vec::with_capacity(produced.len());
        for (offset, source) in produced.iter().enumerate() {
            let index = start + u32::try_from(offset).unwrap_or(0);
            let target = dir.join(series_name(&self.stem, tag, index, &ext));
            if let Err(error) = link_or_rename(source, &target) {
                for done in &published {
                    let _ = fs::remove_file(done);
                }
                return Err(error);
            }
            published.push(target);
        }

        // The reserved single-file scratch was only ever a placeholder for the
        // directory; the series never wrote to it.
        let _ = fs::remove_file(&self.scratch_path);
        self.committed = true;
        Ok(published)
    }

    /// Publish the engine's output under the reserved name.
    ///
    /// Uses a hard link so the publish cannot overwrite a file that appeared
    /// after the name was reserved: a link fails where a rename would silently
    /// replace. On filesystems without links it falls back to a rename, which on
    /// Windows is also a no-clobber operation.
    ///
    /// # Errors
    ///
    /// Fails when the destination appeared in the meantime, or when the result
    /// cannot be moved into place. The scratch file is removed either way.
    pub fn commit(mut self) -> io::Result<PathBuf> {
        // Publish first: if linking fails we still own the scratch file and the
        // `Drop` impl will clear it.
        link_or_rename(&self.scratch_path, &self.final_path)?;
        // The link is the result now; the scratch name goes away. Nothing to do
        // if this fails — the scratch file is a hidden duplicate, not a leak.
        let _ = fs::remove_file(&self.scratch_path);
        self.committed = true;
        Ok(self.final_path.clone())
    }
}

/// Publish one file without ever replacing an existing one.
///
/// A hard link fails where a rename would replace, which closes the window
/// between choosing a name and using it. Filesystems without links fall back to
/// a rename after an explicit existence check; on Windows a rename is itself a
/// no-clobber operation.
fn link_or_rename(from: &Path, to: &Path) -> io::Result<()> {
    match fs::hard_link(from, to) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(error),
        Err(_) => {
            if to.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} already exists", to.display()),
                ));
            }
            fs::rename(from, to)
        }
    }
}

impl Drop for OutputPlan {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.scratch_path);
        }
    }
}

/// `"photo (converted).webp"` for the first variant, `"photo (converted 2).webp"`
/// after that.
fn variant_name(stem: &str, tag: &str, ext: &str, variant: u32) -> String {
    if variant == 1 {
        format!("{stem} ({tag}).{ext}")
    } else {
        format!("{stem} ({tag} {variant}).{ext}")
    }
}

/// `"scan (converted 001).png"` — a series is numbered from one, always with
/// three digits, so the order is obvious in a file manager and sorts as text.
fn series_name(stem: &str, tag: &str, index: u32, ext: &str) -> String {
    format!("{stem} ({tag} {index:03}).{ext}")
}

/// Also used by the planner, which has to name its side files before there is
/// an [`OutputPlan`] to ask.
#[must_use]
pub fn scratch_sibling(scratch: &Path, label: &str) -> PathBuf {
    let name = match scratch.file_stem() {
        Some(stem) => format!("{}.{label}", stem.to_string_lossy()),
        None => format!(".orbit-{label}"),
    };
    scratch.with_file_name(name)
}

/// Create a hidden, unused scratch file that keeps the destination extension.
///
/// The extension is load-bearing: ffmpeg picks its muxer from the output
/// suffix, so a scratch file named `.orbit-1.tmp` would produce a format error
/// instead of an MP3.
fn create_scratch(dir: &Path, ext: &str) -> io::Result<PathBuf> {
    for _ in 0..MAX_VARIANTS {
        let n = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let candidate = dir.join(format!(".orbit-{}-{n}.{ext}", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a scratch file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway directory that removes itself.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("orbit-jobs-{label}-{}", std::process::id()));
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

    fn source(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"input").expect("write source");
        path
    }

    #[test]
    fn name_follows_the_documented_pattern() {
        let dir = TempDir::new("pattern");
        let plan = OutputPlan::reserve(&source(dir.path(), "photo.png"), "converted", "webp", None)
            .unwrap();
        assert_eq!(
            plan.final_path().file_name().unwrap(),
            "photo (converted).webp"
        );
    }

    #[test]
    fn output_lands_beside_the_source() {
        let dir = TempDir::new("beside");
        let plan =
            OutputPlan::reserve(&source(dir.path(), "clip.mov"), "converted", "mp3", None).unwrap();
        assert_eq!(plan.final_path().parent().unwrap(), dir.path());
    }

    #[test]
    fn an_existing_result_is_never_overwritten() {
        let dir = TempDir::new("noclobber");
        let first =
            OutputPlan::reserve(&source(dir.path(), "a.png"), "converted", "webp", None).unwrap();
        let taken = first.final_path().to_path_buf();
        fs::write(&taken, b"already yours").unwrap();

        let second =
            OutputPlan::reserve(&source(dir.path(), "a.png"), "converted", "webp", None).unwrap();
        assert_eq!(
            second.final_path().file_name().unwrap(),
            "a (converted 2).webp"
        );
        assert_eq!(fs::read(&taken).unwrap(), b"already yours");
    }

    #[test]
    fn custom_folder_is_created_and_used() {
        let dir = TempDir::new("folder");
        let out = dir.path().join("nested/out");
        let plan =
            OutputPlan::reserve(&source(dir.path(), "a.png"), "converted", "jpg", Some(&out))
                .unwrap();
        assert_eq!(plan.final_path().parent().unwrap(), out);
        assert!(out.is_dir());
    }

    #[test]
    fn scratch_keeps_the_target_extension() {
        let dir = TempDir::new("scratch-ext");
        // ffmpeg infers its muxer from this suffix, so it must survive.
        let plan =
            OutputPlan::reserve(&source(dir.path(), "song.wav"), "converted", "mp3", None).unwrap();
        assert_eq!(plan.scratch_path().extension().unwrap(), "mp3");
        assert!(
            plan.scratch_path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".orbit-"),
            "scratch files stay hidden"
        );
        assert!(plan.scratch_path().exists(), "scratch is created up front");
    }

    #[test]
    fn commit_publishes_the_result_and_clears_the_scratch() {
        let dir = TempDir::new("commit");
        let plan =
            OutputPlan::reserve(&source(dir.path(), "a.png"), "converted", "webp", None).unwrap();
        let scratch = plan.scratch_path().to_path_buf();
        fs::write(&scratch, b"engine output").unwrap();

        let published = plan.commit().unwrap();
        assert_eq!(fs::read(&published).unwrap(), b"engine output");
        assert!(!scratch.exists(), "scratch name is released");
    }

    #[test]
    fn dropping_without_commit_leaves_nothing_behind() {
        let dir = TempDir::new("drop");
        let plan =
            OutputPlan::reserve(&source(dir.path(), "a.png"), "converted", "webp", None).unwrap();
        let scratch = plan.scratch_path().to_path_buf();
        let would_be = plan.final_path().to_path_buf();
        fs::write(&scratch, b"half a file").unwrap();
        drop(plan);

        assert!(!scratch.exists(), "the failed job is swept up");
        assert!(!would_be.exists(), "and nothing was published");
    }

    #[test]
    fn commit_refuses_a_destination_that_appeared_first() {
        let dir = TempDir::new("race");
        let plan = OutputPlan::reserve(&source(dir.path(), "race.png"), "converted", "webp", None)
            .unwrap();
        let scratch = plan.scratch_path().to_path_buf();
        fs::write(&scratch, b"mine").unwrap();
        // Someone wins the race between reserving the name and publishing.
        fs::write(plan.final_path(), b"theirs").unwrap();

        let error = plan.commit().expect_err("must not clobber");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read(dir.path().join("race (converted).webp")).unwrap(),
            b"theirs"
        );
        assert!(!scratch.exists(), "scratch is still cleaned up");
    }

    #[test]
    fn a_source_without_a_name_is_an_error() {
        let error = OutputPlan::reserve(Path::new("/"), "converted", "webp", None)
            .expect_err("no stem to build on");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_series_is_numbered_from_one_and_published_whole() {
        let dir = TempDir::new("series");
        let mut plan =
            OutputPlan::reserve(&source(dir.path(), "scan.pdf"), "converted", "png", None).unwrap();
        let scratch = plan.scratch_path().to_path_buf();
        let inputs: Vec<PathBuf> = (1..=3)
            .map(|n| {
                let path = dir.path().join(format!("produced-{n}.png"));
                fs::write(&path, format!("page {n}")).unwrap();
                path
            })
            .collect();

        let published = plan.publish_series("converted", "png", &inputs).unwrap();
        let names: Vec<_> = published
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            names,
            [
                "scan (converted 001).png",
                "scan (converted 002).png",
                "scan (converted 003).png"
            ]
        );
        assert_eq!(fs::read(&published[1]).unwrap(), b"page 2");
        assert!(!scratch.exists(), "the placeholder scratch is released");
    }

    #[test]
    fn a_series_moves_as_a_block_when_the_first_name_is_taken() {
        let dir = TempDir::new("series-block");
        let mut plan =
            OutputPlan::reserve(&source(dir.path(), "scan.pdf"), "converted", "png", None).unwrap();
        // Occupy the first slot; the whole run has to start at 002 so the
        // numbering never interleaves with an older result.
        fs::write(dir.path().join("scan (converted 001).png"), b"older").unwrap();
        let inputs: Vec<PathBuf> = (1..=2)
            .map(|n| {
                let path = dir.path().join(format!("produced-block-{n}.png"));
                fs::write(&path, b"new").unwrap();
                path
            })
            .collect();

        let published = plan.publish_series("converted", "png", &inputs).unwrap();
        assert_eq!(
            published
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["scan (converted 002).png", "scan (converted 003).png"]
        );
        assert_eq!(
            fs::read(dir.path().join("scan (converted 001).png")).unwrap(),
            b"older"
        );
    }

    #[test]
    fn an_empty_series_is_refused() {
        let dir = TempDir::new("series-empty");
        let mut plan =
            OutputPlan::reserve(&source(dir.path(), "scan.pdf"), "converted", "png", None).unwrap();
        let error = plan
            .publish_series("converted", "png", &[])
            .expect_err("nothing to publish");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
