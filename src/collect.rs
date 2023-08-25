use std::cmp::Reverse;

use camino::{Utf8Path, Utf8PathBuf};
use clap::ValueEnum;
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
                                let size = entry.metadata()?.len();
                                if let Some(min_size) = self.min_size {
                                    if size <= min_size {
                                        debug!("skipping file {} because it is too small", path);
                                        continue;
                                    }
                                }
                                info!("found video file: {path}");
                                files.push((path.to_owned(), size));
                            }
                        }
                    }
                }
                Err(e) => warn!("error while walking directory: {}", e),
            }
        }

        if let Some(order) = self.order {
            match order {
                FileSortOrder::BiggestFirst => {
                    files.sort_by_key(|(_, size)| Reverse(*size));
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
