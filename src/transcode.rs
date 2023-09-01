use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{fmt, fs};

use camino::Utf8Path;
use console::{Emoji, Term};
use human_repr::HumanCount;
use indicatif::{
    FormattedDuration, MultiProgress, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle,
};
use once_cell::sync::Lazy;
use regex::Regex;
use tracing::{debug, info, warn};

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
    pub progress_hidden: bool,
}

impl From<Args> for TranscodeOptions {
    fn from(args: Args) -> Self {
        Self {
            crf: args.crf,
            effort: args.effort,
            codecs: args.codecs,
            dry_run: args.dry_run,
            replace: args.replace,
            progress_hidden: args.log.is_some(),
        }
    }
}

fn trim_path(path: &Utf8Path) -> String {
    const MAX_LEN: usize = 40;

    if let Some(name) = path.file_name() {
        if name.len() >= MAX_LEN {
            format!("{}…", &name[0..MAX_LEN])
        } else {
            name.into()
        }
    } else {
        "".into()
    }
}

fn ffmpeg_progress_bar(file: &VideoFile, hidden: bool) -> ProgressBar {
    if hidden {
        ProgressBar::hidden()
    } else {
        let style = ProgressStyle::with_template(
            "{msg} {elapsed} {wide_bar:.cyan/blue} Transcoded {pos_duration} / {len_duration}, ETA: {eta}",
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
        );
        ProgressBar::new((file.duration * 1000.0) as u64)
            .with_style(style)
            .with_message(format!("Transcoding file '{}'", trim_path(&file.path),))
    }
}

pub struct Transcoder {
    options: TranscodeOptions,
    files: Vec<VideoFile>,
    progress: MultiProgress,
}

impl Transcoder {
    pub fn new(options: TranscodeOptions, files: Vec<VideoFile>) -> Self {
        info!("Transcoding files with options {options:?}");
        let progress = MultiProgress::new();
        if options.progress_hidden {
            progress.set_draw_target(ProgressDrawTarget::hidden());
        }
        Self {
            options,
            files,
            progress,
        }
    }

    fn print_file_list(&self, term: &MultiProgress, completed_index: usize) -> Result<()> {
        for (index, file) in self.files.iter().enumerate() {
            let string = if index == completed_index {
                format!("[{}] {}", Emoji("⚒️", "..."), trim_path(&file.path))
            } else if index < completed_index {
                format!("[{}] {}", Emoji("✅", "✓"), trim_path(&file.path))
            } else {
                format!("[ ] {}", trim_path(&file.path))
            };

            term.println(&string)?;
        }
        Ok(())
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
                    if s.contains(' ') {
                        format!("\"{}\"", s)
                    } else {
                        s.to_string()
                    }
                })
                .collect();
            let args = args.join(" ");

            info!(
                "Would transcode file '{}' with size {}",
                file.path.file_name().expect("file must have a name"),
                file.file_size.human_count_bytes()
            );
            debug!("Command to run: ffmpeg {}", args);
            return Ok(());
        }

        let mut process = Command::new("ffmpeg")
            .args(args)
            .stderr(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()?;

        let stdout = process.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        let file_name = trim_path(&file.path);
        info!("Transcoding file {}", file_name);
        let progress = self
            .progress
            .add(ffmpeg_progress_bar(file, self.options.progress_hidden));
        progress.tick();
        let mut last_postion = 0;
        for line in reader.lines() {
            let line = line?;
            debug!("{}", line);
            if let Some(captures) = OUT_TIME_REGEX.captures(&line) {
                let duration: u64 = captures.get(1).unwrap().as_str().parse::<u64>()?;
                let duration = Duration::from_micros(duration);
                let millis = duration.as_millis() as u64;
                info!(
                    "{}: {} / {}",
                    file_name,
                    millis,
                    (file.duration * 1000.0) as u64
                );
                let delta = millis - last_postion;
                progress.inc(delta);
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
        let term = Term::stderr();
        if !self.options.progress_hidden {
            term.clear_screen()?;
            term.hide_cursor()?;
        }

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

        let progress = self.progress.add(if self.options.progress_hidden {
            ProgressBar::hidden()
        } else {
            ProgressBar::new(total_duration).with_style(
                ProgressStyle::default_bar()
                    .template("Total progress: {wide_bar:.cyan/blue} {eta}")?,
            )
        });
        progress.tick();
        for (index, file) in filtered_files.into_iter().enumerate() {
            self.print_file_list(&self.progress, index)?;
            match self.transcode_file(file, &progress) {
                Ok(_) => {}
                Err(e) => {
                    warn!("Could not transcode file {}: {:?}", file.path, e);
                }
            }
        }

        Ok(())
    }
}
