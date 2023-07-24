use std::io::{BufRead, BufReader};
use std::process::{ChildStderr, Command};
use std::time::Instant;

use color_eyre::eyre::bail;
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
    let stem = file.path.file_stem().expect("file must have a name");
    let out_path = file.path.with_file_name(format!("{}_av1.mp4", stem));
    if out_path.is_file() {
        info!("File {} already exists, skipping", out_path.as_str());
        return Ok(());
    }
    let effort = options.effort.to_string();
    let crf = options.crf.to_string();
    let args = vec![
        "-i",
        file.path.as_str(),
        "-c:v",
        "libsvtav1",
        "-preset",
        &effort,
        "-crf",
        &crf,
        "-c:a",
        "copy",
        "-progress",
        "-",
        "-nostats",
        out_path.as_str(),
    ];
    if options.dry_run {
        let args: Vec<_> = args
            .iter()
            .map(|s| {
                if s.contains(" ") {
                    format!("\"{}\"", s)
                } else {
                    s.to_string()
                }
            })
            .collect();
        let args = args.join(" ");

        info!(
            "Would transcode file {} with size {} and command 'ffmpeg {}'",
            file.path.as_str(),
            file.file_size.human_count_bytes(),
            args
        );
        return Ok(());
    }

    let start = Instant::now();
    let process: std::process::Child = Command::new("ffmpeg").args(args).spawn()?;
    if let Some(stdout) = process.stdout {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = line?;
        }
    }
    let status = process.wait()?;

    if status.success() {
        let elapsed = start.elapsed();
        let new_size = out_path.metadata()?.len();
        Ok(())
    } else {
        bail!("ffmpeg failed");
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
