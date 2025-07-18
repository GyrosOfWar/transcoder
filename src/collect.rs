use std::borrow::Cow;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use clap::ValueEnum;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use tracing::{debug, info, warn};
use walkdir::{DirEntry, WalkDir};

use crate::database::{Database, NewTranscodeFile, TranscodeFile};
use crate::ffprobe::ffprobe;
use crate::Result;

fn file_name_short(path: &Utf8Path, len: usize) -> Cow<'_, str> {
    let name = path.file_name().unwrap_or_default();
    if name.len() > len {
        Cow::Owned(name.chars().take(len - 1).collect::<String>() + "...")
    } else {
        Cow::Borrowed(name)
    }
}

#[derive(Debug, Clone)]
pub struct VideoFile {
    pub rowid: i64,
    pub path: Utf8PathBuf,
    /// Duration in seconds.
    pub duration: f64,
    pub resolution: (u32, u32),
    pub bitrate: u64,
    pub frame_rate: f64,
    pub codec: String,
    pub file_size: u64,
}

impl From<TranscodeFile> for VideoFile {
    fn from(value: TranscodeFile) -> Self {
        let info = value.ffprobe().expect("ffprobe info must be present");
        VideoFile {
            rowid: value.rowid,
            path: value.path,
            duration: info.duration().unwrap_or_default(),
            resolution: info.resolution(),
            bitrate: info.bitrate(),
            frame_rate: info.frame_rate(),
            codec: info.video_codec().to_owned(),
            file_size: value.file_size as u64,
        }
    }
}

impl VideoFile {
    pub fn difficulty(&self) -> u64 {
        let (width, height) = self.resolution;
        (width * height) as u64 * self.duration as u64 * self.bitrate * self.frame_rate as u64
    }
}

const EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v"];

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FileSortOrder {
    BiggestFirst,
}

pub struct Collector {
    database: Database,

    exclude: Vec<String>,
    base_path: Utf8PathBuf,
    min_size: Option<u64>,
}

impl Collector {
    pub fn new(
        database: Database,
        base_path: Utf8PathBuf,
        exclude: Vec<String>,
        min_size: Option<u64>,
    ) -> Self {
        Self {
            database,
            exclude,
            base_path,
            min_size,
        }
    }

    fn is_excluded(&self, e: &DirEntry) -> bool {
        let path = Utf8Path::from_path(e.path()).expect("path must be utf-8");
        let is_excluded = self.exclude.iter().any(|p| path.as_str().contains(p));
        debug!("{} is excluded: {}", path, is_excluded);
        is_excluded
    }

    pub fn gather_files(&self) -> Result<Vec<Utf8PathBuf>> {
        let progress = ProgressBar::new_spinner();
        progress.set_message("Gathering files...");
        progress.enable_steady_tick(Duration::from_millis(250));

        info!("gathering files at {}", self.base_path);
        if self.base_path.is_file() {
            info!("path argument is a file, not a directory, returning it");
            return Ok(vec![self.base_path.clone()]);
        }
        let mut files = vec![];
        let walker = WalkDir::new(&self.base_path).into_iter();
        for entry in walker.filter_entry(|e| !self.is_excluded(e)) {
            match entry {
                Ok(entry) => {
                    if entry.file_type().is_file() {
                        let path = Utf8Path::from_path(entry.path()).expect("path must be utf-8");
                        if let (Some(stem), Some(ext)) = (path.file_stem(), path.extension()) {
                            if EXTENSIONS.contains(&ext) && !stem.ends_with("_tmp") {
                                match path.metadata() {
                                    Ok(metadata) => {
                                        let size = metadata.len();
                                        if let Some(min_size) = self.min_size {
                                            if size <= min_size {
                                                debug!(
                                                    "skipping file {} because it is too small",
                                                    path
                                                );
                                                continue;
                                            }
                                        }
                                        info!("found video file: {path}");

                                        files.push((path.to_owned(), size));
                                    }
                                    Err(e) => {
                                        warn!("skipping file {} because of error: {}", path, e)
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => warn!("error while walking directory: {}", e),
            }
        }
        progress.finish_and_clear();

        let progress = ProgressBar::new(files.len() as u64).with_style(
            ProgressStyle::default_bar().template("{msg} {wide_bar:.cyan/blue} {eta}")?,
        );
        progress.tick();

        let mut files: Vec<_> = files
            .into_par_iter()
            .flat_map(|(path, size)| ffprobe(&path).map(|ffprobe| (path, ffprobe, size)))
            .inspect(|p| {
                let name = file_name_short(&p.0, 40);
                progress.set_message(format!("Processing {:40}", name));
                progress.inc(1);
            })
            .collect();

        progress.finish_and_clear();

        let excluded_codecs = &["hevc", "av1"];
        files.retain(|(_, ffprobe, _)| !excluded_codecs.contains(&ffprobe.video_codec()));

        info!("gathered {} files", files.len());

        let records: Vec<_> = files
            .iter()
            .map(|f| NewTranscodeFile {
                file_size: f.2,
                path: f.0.clone(),
                ffprobe_info: f.1.clone(),
            })
            .collect();
        self.database.insert_batch(&records)?;

        Ok(files.into_iter().map(|f| f.0).collect())
    }
}
