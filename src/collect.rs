use std::cmp::Reverse;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use clap::ValueEnum;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use tracing::{debug, info, warn};
use walkdir::{DirEntry, WalkDir};

use crate::ffprobe::ffprobe;
use crate::Result;

#[derive(Debug, Clone)]
pub struct VideoFile {
    pub path: Utf8PathBuf,
    /// Duration in seconds.
    pub duration: f64,
    pub resolution: (u32, u32),
    pub bitrate: u64,
    pub frame_rate: f64,
    pub codec: String,
    pub file_size: u64,
}

impl VideoFile {
    #[allow(unused)]
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
    exclude: Vec<String>,
    base_path: Utf8PathBuf,
    min_size: Option<u64>,
    order: Option<FileSortOrder>,
    count: Option<usize>,
}

impl Collector {
    pub fn new(
        base_path: Utf8PathBuf,
        exclude: Vec<String>,
        min_size: Option<u64>,
        order: Option<FileSortOrder>,
        count: Option<usize>,
    ) -> Self {
        Self {
            exclude,
            base_path,
            min_size,
            order,
            count,
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

        let progress = ProgressBar::new(files.len() as u64)
            .with_style(ProgressStyle::default_bar().template("{wide_bar:.cyan/blue} {eta}")?);
        progress.tick();

        let mut files: Vec<_> = files
            .into_par_iter()
            .flat_map(|(path, size)| ffprobe(&path).map(|ffprobe| (path, ffprobe, size)))
            .inspect(|_| progress.inc(1))
            .collect();

        progress.finish_and_clear();

        let excluded_codecs = &["hevc", "av1"];
        files.retain(|(_, ffprobe, _)| !excluded_codecs.contains(&ffprobe.video_codec()));

        info!("gathered {} files", files.len());
        if let Some(order) = self.order {
            match order {
                FileSortOrder::BiggestFirst => {
                    files.sort_by_key(|(_, _, size)| Reverse(*size));
                }
            }
        }

        if let Some(count) = self.count {
            files.truncate(count);
        }

        Ok(files.into_iter().map(|f| f.0).collect())
    }

    pub fn probe_files(&self, files: Vec<Utf8PathBuf>) -> Result<Vec<VideoFile>> {
        let results: Vec<_> = files
            .into_par_iter()
            .filter_map(|f| {
                let result = ffprobe(&f).ok()?;
                Some(VideoFile {
                    path: f,
                    duration: result.duration().unwrap_or_default(),
                    resolution: result.resolution(),
                    bitrate: result.bitrate(),
                    frame_rate: result.frame_rate(),
                    codec: result.video_codec().to_owned(),
                    file_size: result.size(),
                })
            })
            .collect();

        Ok(results)
    }
}
