use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{fmt, fs};

use camino::Utf8Path;
use console::Term;
use human_repr::HumanCount;
use indicatif::{FormattedDuration, MultiProgress, ProgressBar, ProgressState, ProgressStyle};
use once_cell::sync::Lazy;
use regex::Regex;
use tracing::{info, warn};

use crate::collect::VideoFile;
use crate::ffprobe::commandline_error;
use crate::{Args, Result};

static OUT_TIME_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"out_time_us=(\d+)").unwrap());

#[derive(Debug, Clone)]
pub struct TranscodeOptions {
    pub crf: u8,
    pub effort: u8,
    pub codecs: Vec<String>,
    pub dry_run: bool,
    pub replace: bool,
}

impl From<Args> for TranscodeOptions {
    fn from(args: Args) -> Self {
        Self {
            crf: args.crf,
            effort: args.effort,
            codecs: args.codecs,
            dry_run: args.dry_run,
            replace: args.replace,
        }
    }
}

fn trim_path<'a>(path: &'a Utf8Path) -> &'a str {
    const MAX_LEN: usize = 200;

    if let Some(name) = path.file_name() {
        if name.len() >= MAX_LEN {
            &name[0..MAX_LEN]
        } else {
            name
        }
    } else {
        ""
    }
}

fn ffmpeg_progress_bar(file: &VideoFile) -> ProgressBar {
    ProgressBar::new((file.duration * 1000.0) as u64).with_style(
        ProgressStyle::with_template(
            "{msg} {elapsed} {wide_bar:.cyan/blue} Trancoded {pos_duration} / {len_duration}, ETA: {eta}",
        )
        .unwrap()
        .with_key(
            "pos_duration",
            |state: &ProgressState, w: &mut dyn fmt::Write| {
                write!(
                    w,
                    "{}",
                    FormattedDuration(Duration::from_millis(state.pos()))
                )
                .unwrap()
            },
        )
        .with_key(
            "len_duration",
            |state: &ProgressState, w: &mut dyn fmt::Write| {
                write!(
                    w,
                    "{}",
                    FormattedDuration(Duration::from_millis(state.len().unwrap()))
                )
                .unwrap()
            },
        ),
    )
    .with_message(format!(
        "Transcoding file '{}'",
        trim_path(&file.path),
    ))
}

pub struct Transcoder {
    options: TranscodeOptions,
    files: Vec<VideoFile>,
    progress: MultiProgress,
}

impl Transcoder {
    pub fn new(options: TranscodeOptions, files: Vec<VideoFile>) -> Self {
        Self {
            options,
            files,
            progress: MultiProgress::new(),
        }
    }

    fn transcode_file(&self, file: &VideoFile, total_progress: &ProgressBar) -> Result<()> {
        let stem = file.path.file_stem().expect("file must have a name");
        let out_file = file.path.with_file_name(format!("{stem}_av1.mp4"));
        if out_file.is_file() {
            info!("File {} already exists, skipping", out_file.as_str());
            return Ok(());
        }
        let tmp_file = file.path.with_file_name(format!("{stem}_tmp.mp4"));
        let effort = self.options.effort.to_string();
        let crf = self.options.crf.to_string();
        let args = vec![
            "-y",
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
            tmp_file.as_str(),
        ];
        if self.options.dry_run {
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

        let mut process = Command::new("ffmpeg")
            .args(args)
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;

        let stdout = process.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        let progress = self.progress.add(ffmpeg_progress_bar(file));
        let mut last_postion = 0;
        for line in reader.lines() {
            let line = line?;
            if let Some(captures) = OUT_TIME_REGEX.captures(&line) {
                let duration: u64 = captures.get(1).unwrap().as_str().parse::<u64>()?;
                let duration = Duration::from_micros(duration);
                let millis = duration.as_millis() as u64;
                let delta = millis - last_postion;
                progress.set_position(millis);
                total_progress.inc(delta);
                last_postion = millis;
            }
        }
        progress.finish_and_clear();

        let output = process.wait_with_output()?;
        if output.status.success() {
            if self.options.replace {
                fs::remove_file(&file.path)?;
                fs::rename(tmp_file, &file.path)?;
            } else {
                fs::rename(tmp_file, out_file)?;
            }
            Ok(())
        } else {
            commandline_error("ffmpeg", output)
        }
    }

    pub fn transcode_all(&self) -> Result<()> {
        let term = Term::stdout();
        term.clear_screen()?;

        let filtered_files: Vec<_> = self
            .files
            .iter()
            .filter(|f| self.options.codecs.contains(&f.codec))
            .collect();
        let len = filtered_files.len();
        info!("transcoding {len} files (codecs {:?})", self.options.codecs);

        let total_duration = filtered_files
            .iter()
            .map(|f| Duration::from_secs_f64(f.duration).as_millis() as u64)
            .sum();

        let progress = self.progress.add(
            ProgressBar::new(total_duration).with_style(
                ProgressStyle::default_bar()
                    .template("Total progress {pos} / {len} {wide_bar:.cyan/blue} {eta}")?,
            ),
        );
        for file in filtered_files {
            match self.transcode_file(&file, &progress) {
                Ok(_) => {}
                Err(e) => {
                    warn!("Could not transcode file {}: {:?}", file.path, e);
                }
            }
        }

        Ok(())
    }
}
