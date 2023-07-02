use std::process::Command;

use indicatif::ProgressIterator;
use tracing::{warn, info};

use crate::database::VideoFile;
use crate::ffprobe::commandline_error;
use crate::{Result, Args};

#[derive(Debug, Clone)]
pub struct TranscodeOptions {
    pub crf: u8,
    pub effort: u8,
    pub codecs: Vec<String>,
}

impl From<Args> for TranscodeOptions {
    fn from(args: Args) -> Self {
        Self {
            crf: args.crf,
            effort: args.effort,
            codecs: args.codecs,
        }
    }
}

fn transcode_file(file: &VideoFile, options: &TranscodeOptions) -> Result<()> {
    let stem = file.path.file_stem().expect("file must have a name");
    let out_path = file.path.with_file_name(format!("{}_av1.mp4", stem));
    let output = Command::new("ffmpeg")
        .args(vec![
            "-i",
            file.path.as_str(),
            "-c:v",
            "libsvtav1",
            "-crf",
            &options.crf.to_string(),
            "-preset",
            &options.effort.to_string(),
            "-c:a",
            "copy",
            out_path.as_str(),
        ])
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        commandline_error("ffmpeg", output)
    }
}

pub fn transcode_all(files: Vec<VideoFile>, options: TranscodeOptions) -> Result<()> {
    let filtered_files: Vec<_> = files
        .into_iter()
        .filter(|f| options.codecs.contains(&f.codec))
        .collect();
    let len = filtered_files.len() as u64;

    for file in filtered_files.into_iter().progress_count(len) {
        match transcode_file(&file, &options) {
            Ok(_) => {
                // info!("transcoded file {}", file.path);
            }
            Err(e) => {
                warn!("Could not transcode file {}: {:?}", file.path, e);
            }
        }
    }

    Ok(())
}
