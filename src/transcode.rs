use std::process::Command;
use std::time::Instant;

use human_repr::HumanCount;
use tracing::{info, warn};

use crate::collect::VideoFile;
use crate::ffprobe::commandline_error;
use crate::{Args, Result};

#[derive(Debug, Clone)]
pub struct TranscodeOptions {
    pub crf: u8,
    pub effort: u8,
    pub codecs: Vec<String>,
    pub dry_run: bool,
}

impl From<Args> for TranscodeOptions {
    fn from(args: Args) -> Self {
        Self {
            crf: args.crf,
            effort: args.effort,
            codecs: args.codecs,
            dry_run: args.dry_run,
        }
    }
}

// #[instrument]
fn transcode_file(file: &VideoFile, options: &TranscodeOptions) -> Result<()> {
    if options.dry_run {
        info!(
            "Would transcode file {} with size {}",
            file.path.as_str(),
            file.file_size.human_count_bytes()
        );
        return Ok(());
    }

    let start = Instant::now();
    let stem = file.path.file_stem().expect("file must have a name");
    let out_path = file.path.with_file_name(format!("{}_av1.mp4", stem));
    if out_path.is_file() {
        info!("File {} already exists, skipping", out_path.as_str());
        return Ok(());
    }
    let output = Command::new("ffmpeg")
        .args(vec![
            "-i",
            file.path.as_str(),
            "-c:v",
            "libsvtav1",
            "-preset",
            &options.effort.to_string(),
            "-crf",
            &options.crf.to_string(),
            "-c:a",
            "copy",
            out_path.as_str(),
        ])
        .output()?;
    if output.status.success() {
        let elapsed = start.elapsed();
        let new_size = out_path.metadata()?.len();
        info!(
            "Transcoded file {} with duration {}s and resolution {}x{} in {}s. Initial size: {}, new size: {}",
            file.path.file_name().unwrap(),
            file.duration,
            file.resolution.0,
            file.resolution.1,
            elapsed.as_secs_f32(),
            file.file_size.human_count_bytes(),
            new_size.human_count_bytes()
        );
        Ok(())
    } else {
        commandline_error("ffmpeg", output)
    }
}

pub struct Transcoder {}

impl Transcoder {
    pub fn new() -> Self {
        Self {}
    }

    pub fn transcode_all(&self, files: Vec<VideoFile>, options: TranscodeOptions) -> Result<()> {
        let filtered_files: Vec<_> = files
            .into_iter()
            .filter(|f| options.codecs.contains(&f.codec))
            .collect();
        let len = filtered_files.len() as u64;
        info!("transcoding {len} files (codecs {:?})", options.codecs);

        for file in filtered_files.into_iter() {
            match transcode_file(&file, &options) {
                Ok(_) => {}
                Err(e) => {
                    warn!("Could not transcode file {}: {:?}", file.path, e);
                }
            }
        }

        Ok(())
    }
}
