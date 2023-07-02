use std::process::Command;
use std::time::Instant;

use tracing::{info, instrument, warn};

use crate::database::{Database, VideoConversion, VideoFile};
use crate::ffprobe::commandline_error;
use crate::{Args, Result};

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

#[instrument]
fn transcode_file(file: &VideoFile, options: &TranscodeOptions) -> Result<()> {
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
            "-svtav1-params",
            "mbr=3000k",
            "-g",
            "240",
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
        info!(
            "Transcoded file {} with duration {}s in {}s",
            file.path.as_str(),
            file.duration,
            elapsed.as_secs_f32()
        );
        Ok(())
    } else {
        commandline_error("ffmpeg", output)
    }
}

pub struct Transcoder {
    db: Database,
}

impl Transcoder {
    pub fn new(db: Database) -> Self {
        Self { db }
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
                Ok(_) => {
                    self.db.add_video_conversion(VideoConversion {
                        video_file_id: file.rowid.expect("file must have rowid set"),
                        original_codec: file.codec,
                        new_codec: "av1".to_string(),
                        created_at: None,
                        updated_at: None,
                    })?;
                }
                Err(e) => {
                    warn!("Could not transcode file {}: {:?}", file.path, e);
                }
            }
        }

        Ok(())
    }
}
