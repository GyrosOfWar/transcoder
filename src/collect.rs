use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser;
use indicatif::ParallelProgressIterator;
use rayon::prelude::{IntoParallelIterator, ParallelIterator};
use tracing::{debug, info};
use walkdir::{DirEntry, WalkDir};

use crate::database::{Database, VideoFile};
use crate::ffprobe::ffprobe;
use crate::Result;

const EXTENSIONS: &[&str] = &["mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v"];

fn is_excluded(e: &DirEntry, exclude: &[String]) -> bool {
    let path = Utf8Path::from_path(e.path()).expect("path must be utf-8");
    let is_excluded = exclude.iter().any(|p| path.as_str().contains(p));
    debug!("{} is excluded: {}", path, is_excluded);
    is_excluded
}

pub fn gather_files(
    base_path: impl AsRef<Utf8Path>,
    exclude: Vec<String>,
) -> Result<Vec<Utf8PathBuf>> {
    info!("gathering files at {}", base_path.as_ref());
    let mut files = vec![];
    let path = base_path.as_ref().as_std_path();
    let walker = WalkDir::new(path).into_iter();
    for entry in walker.filter_entry(|e| !is_excluded(e, &exclude)) {
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

pub fn probe_files(files: Vec<Utf8PathBuf>) -> Vec<VideoFile> {
    let len = files.len() as u64;
    files
        .into_par_iter()
        .progress_count(len)
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
            })
        })
        .collect()
}
