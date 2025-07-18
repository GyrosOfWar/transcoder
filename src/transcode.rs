use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{fmt, fs};

use camino::Utf8Path;
use clap::ValueEnum;
use console::{Emoji, Term};
use human_repr::HumanCount;
use indicatif::{
    FormattedDuration, MultiProgress, ProgressBar, ProgressDrawTarget, ProgressState, ProgressStyle,
};
use once_cell::sync::Lazy;
use rayon::prelude::*;
use rayon::ThreadPoolBuilder;
use regex::Regex;
use tracing::{debug, info, warn};

use crate::collect::VideoFile;
use crate::database::{Database, TranscodeStatus};
use crate::ffprobe::commandline_error;
use crate::Result;

static OUT_TIME_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"out_time_us=(\d+)").unwrap());

#[derive(Debug, Clone, ValueEnum)]
pub enum GpuMode {
    Nvidia,
    Qsv,
}

#[derive(Debug, Clone)]
pub struct TranscodeOptions {
    pub crf: u8,
    pub effort: u8,
    pub dry_run: bool,
    pub replace: bool,
    pub progress_hidden: bool,
    pub ignored_codecs: Vec<String>,
    pub gpu: Option<GpuMode>,
    pub parallel: u32,
}

fn trim_path(path: &Utf8Path) -> String {
    const MAX_LEN: usize = 65;

    if let Some(name) = path.file_name() {
        if name.len() >= MAX_LEN {
            format!("{}…", name.chars().take(MAX_LEN - 1).collect::<String>())
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
    database: Database,
}

impl Transcoder {
    pub fn new(database: Database, options: TranscodeOptions, files: Vec<VideoFile>) -> Self {
        info!("Transcoding files with options {options:?}");
        let progress = MultiProgress::new();
        if options.progress_hidden {
            progress.set_draw_target(ProgressDrawTarget::hidden());
        }
        Self {
            database,
            options,
            files,
            progress,
        }
    }

    fn print_file_list(&self, term: &MultiProgress, completed_index: usize) -> Result<()> {
        for (index, file) in self.files.iter().enumerate() {
            let size = file.file_size.human_count_bytes();
            let string = if index == completed_index {
                format!(
                    "[{}] {} ({})",
                    Emoji("⚒️", "..."),
                    trim_path(&file.path),
                    size
                )
            } else if index < completed_index {
                format!(
                    "[{}] {} ({})",
                    Emoji("✅", "✓"),
                    trim_path(&file.path),
                    size
                )
            } else {
                format!("[ ] {} ({})", trim_path(&file.path), size)
            };

            term.println(&string)?;
        }
        Ok(())
    }

    fn transcode_file(&self, file: &VideoFile, total_progress: &ProgressBar) -> Result<()> {
        let progress = self
            .progress
            .add(ffmpeg_progress_bar(file, self.options.progress_hidden));
        let stem = file.path.file_stem().expect("file must have a name");
        let out_file = file.path.with_file_name(format!("{stem}_av1.mp4"));
        if out_file.is_file() {
            info!("File {} already exists, skipping", out_file.as_str());
            return Ok(());
        }
        let tmp_file = file.path.with_file_name(format!("{stem}_tmp.mp4"));
        let effort = match self.options.gpu {
            Some(GpuMode::Nvidia) => format!("p{}", self.options.effort),
            Some(GpuMode::Qsv) | None => self.options.effort.to_string(),
        };
        let crf = self.options.crf.to_string();
        let args = match self.options.gpu {
            Some(GpuMode::Nvidia) => {
                vec![
                    "-y",
                    "-i",
                    file.path.as_str(),
                    "-c:v",
                    "av1_nvenc",
                    "-preset",
                    "p7",
                    "-tune",
                    "hq",
                    "-cq",
                    &crf,
                    "-rc-lookahead",
                    "24",
                    "-b_adapt",
                    "1",
                    "-temporal-aq",
                    "1",
                    "-spatial-aq",
                    "1",
                    "-c:a",
                    "copy",
                    "-progress",
                    "-",
                    "-nostats",
                    tmp_file.as_str(),
                ]
            }
            Some(GpuMode::Qsv) => {
                vec![
                    "-hwaccel",
                    "qsv",
                    "-y",
                    "-i",
                    file.path.as_str(),
                    "-c:v",
                    "av1_qsv",
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
                ]
            }
            None => {
                vec![
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
                ]
            }
        };
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
            info!("Command to run: ffmpeg {}", args);
            progress.tick();
            progress.finish_and_clear();
            total_progress.inc((file.duration * 1000.0) as u64);
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
            let new_file_size = fs::metadata(&tmp_file)?.len();
            info!(
                "Transcoded file {} to size {} from {}",
                file_name,
                new_file_size.human_count_bytes(),
                file.file_size.human_count_bytes()
            );

            if new_file_size >= file.file_size {
                warn!(
                    "Transcoded file {} is larger than original, skipping",
                    file_name
                );
                fs::remove_file(tmp_file)?;
                return Ok(());
            }

            if self.options.replace {
                fs::remove_file(&file.path)?;
                fs::rename(tmp_file, &file.path)?;
            } else {
                fs::rename(tmp_file, out_file)?;
            }

            self.database
                .set_file_status(file.rowid, TranscodeStatus::Success, None)?;
            Ok(())
        } else {
            let error = commandline_error("ffmpeg", output);
            self.database.set_file_status(
                file.rowid,
                TranscodeStatus::Error,
                Some(error.to_string()),
            )?;

            Err(error)
        }
    }

    pub fn transcode_all(&self) -> Result<()> {
        let pool = ThreadPoolBuilder::new()
            .num_threads(self.options.parallel as usize)
            .build()?;
        let term = Term::stderr();
        if !self.options.progress_hidden {
            term.clear_screen()?;
            term.hide_cursor()?;
        }

        pool.install(|| {
            let len = self.files.len();
            info!("transcoding {len} files");

            let total_duration = self
                .files
                .iter()
                .map(|f| Duration::from_secs_f64(f.duration).as_millis() as u64)
                .sum();

            let total_progress = self.progress.add(if self.options.progress_hidden {
                ProgressBar::hidden()
            } else {
                ProgressBar::new(total_duration).with_style(
                    ProgressStyle::default_bar()
                        .template("Total progress: {wide_bar:.cyan/blue} {eta}")
                        .expect("bad progressbar template"),
                )
            });
            total_progress.tick();

            self.files.par_iter().enumerate().for_each(|(_, file)| {
                match self.transcode_file(file, &total_progress) {
                    Ok(_) => {}
                    Err(e) => {
                        warn!("Could not transcode file {}: {:?}", file.path, e);
                    }
                }
            });
        });
        Ok(())
    }
}
