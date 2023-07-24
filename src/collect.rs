use camino::{Utf8Path, Utf8PathBuf};
use indicatif::ParallelProgressIterator;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use tracing::{debug, info};
use walkdir::{DirEntry, WalkDir};

use crate::ffprobe::ffprobe;
use crate::Result;

#[derive(Debug, Clone)]
pub struct VideoFile {
    pub rowid: Option<i64>,
    pub path: Utf8PathBuf,
    pub duration: f64,
    pub resolution: (u32, u32),
    pub bitrate: u64,
    pub frame_rate: f64,
    pub codec: String,
    pub file_size: u64,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

const EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v"];

pub struct Collector {
    exclude: Vec<String>,
    base_path: Utf8PathBuf,
    min_size: Option<u64>,
}

impl Collector {
    pub fn new(base_path: Utf8PathBuf, exclude: Vec<String>, min_size: Option<u64>) -> Self {
        Self {
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
        info!("gathering files at {}", self.base_path);
        if self.base_path.is_file() {
            info!("path argument is a file, not a directory, returning it");
            return Ok(vec![self.base_path.clone()]);
        }
        let mut files = vec![];
        let walker = WalkDir::new(&self.base_path).into_iter();
        for entry in walker.filter_entry(|e| !self.is_excluded(e)) {
            let entry = entry?;
            if entry.file_type().is_file() {
                let path = Utf8Path::from_path(entry.path()).expect("path must be utf-8");
                if let Some(ext) = path.extension() {
                    if EXTENSIONS.contains(&ext) {
                        if let Some(min_size) = self.min_size {
                            let size = entry.metadata()?.len();
                            if size < min_size {
                                debug!("skipping file {} because it is too small", path);
                                continue;
                            }
                        }
                        info!("found video file: {path}");
                        files.push(path.to_owned())
                    }
                }
            }
        }
        Ok(files)
    }

    pub fn probe_files(&self, files: Vec<Utf8PathBuf>) -> Result<Vec<VideoFile>> {
        let len = files.len() as u64;
        let results: Result<Vec<_>> = files
            .into_par_iter()
            .progress_count(len)
            .map(|f| {
                let result = ffprobe(&f);
                result.map(|result| {
                    let size = result
                        .format
                        .size
                        .as_ref()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or_default();
                    VideoFile {
                        rowid: None,
                        path: f.to_owned(),
                        duration: result.duration().unwrap_or_default(),
                        resolution: result.resolution(),
                        bitrate: result.bitrate(),
                        frame_rate: result.frame_rate(),
                        codec: result.video_codec().to_owned(),
                        file_size: size,
                        created_at: None,
                        updated_at: None,
                    }
                })
            })
            .collect();

        Ok(results?)
    }
}
