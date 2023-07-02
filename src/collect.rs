use camino::{Utf8Path, Utf8PathBuf};
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use tracing::{debug, info};
use walkdir::{DirEntry, WalkDir};

use crate::database::{Database, VideoFile};
use crate::ffprobe::ffprobe;
use crate::Result;

const EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v"];

pub struct Collector {
    db: Database,
    exclude: Vec<String>,
    base_path: Utf8PathBuf,
}

impl Collector {
    pub fn new(db: Database, base_path: Utf8PathBuf, exclude: Vec<String>) -> Self {
        Self {
            db,
            exclude,
            base_path,
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
        let mut files = vec![];
        let walker = WalkDir::new(&self.base_path).into_iter();
        for entry in walker.filter_entry(|e| !self.is_excluded(e)) {
            let entry = entry?;
            if entry.file_type().is_file() {
                let path = Utf8Path::from_path(entry.path()).expect("path must be utf-8");
                if let Some(ext) = path.extension() {
                    if EXTENSIONS.contains(&ext) {
                        info!("found video file: {path}");
                        files.push(path.to_owned())
                    }
                }
            }
        }
        Ok(files)
    }

    pub fn probe_files(&self, mut files: Vec<Utf8PathBuf>) -> Result<Vec<VideoFile>> {
        let mut database_files = self.db.get_files_with_path_prefix(&self.base_path)?;
        // remove files that are already in the database
        files.retain(|f| !database_files.iter().any(|r| r.path == *f));

        let results: Result<Vec<_>> = files
            .into_par_iter()
            // .progress_count(len)
            .flat_map(|f| {
                let result = ffprobe(&f).ok()?;
                let size = result
                    .format
                    .size
                    .as_ref()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or_default();
                Some(VideoFile {
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
                })
            })
            .map(|file| self.db.insert_file(&file))
            .collect();

        database_files.extend(results?);
        Ok(database_files)
    }
}
