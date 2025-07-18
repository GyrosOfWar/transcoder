use std::collections::HashMap;
use std::time::Instant;

use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use collect::VideoFile;
use human_repr::{HumanCount, HumanDuration};
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::collect::Collector;
use crate::database::Database;
use crate::transcode::{GpuMode, TranscodeOptions, Transcoder};

mod collect;
mod database;
mod ffprobe;
mod transcode;

pub type Result<T, E = color_eyre::Report> = std::result::Result<T, E>;

#[derive(Subcommand, Debug)]
pub enum Command {
    Scan {
        /// Exclude files that contain this string
        #[clap(short = 'E', long)]
        exclude: Vec<String>,
        /// Minimum file size to transcode
        #[clap(long)]
        min_size: Option<String>,

        /// The path to scan for video files
        path: Utf8PathBuf,
    },
    Transcode {
        /// Limit how many files to process
        #[clap(short, long)]
        number: Option<i64>,

        /// CRF value to use for encoding
        #[clap(short, long, default_value = "24")]
        crf: u8,

        /// Effort level to use for encoding
        #[clap(short, long, default_value = "7")]
        effort: u8,

        /// Dry run, don't do anything
        #[clap(short, long)]
        dry_run: bool,

        #[clap(short, long)]
        replace: bool,

        /// Use the GPU for transcoding
        #[clap(long)]
        gpu: Option<GpuMode>,

        /// Number of files to process in parallel.
        #[clap(short, long, default_value = "1")]
        parallel: u32,
    },
    Info,
}

#[derive(Parser, Debug)]
pub struct Args {
    /// Set the log level
    #[clap(short, long)]
    pub log: Option<tracing::level_filters::LevelFilter>,

    #[clap(subcommand)]
    pub command: Command,
}

fn parse_bytes(string: &str) -> Option<u64> {
    let mut value = string.trim().to_string();
    let suffix = value.split_off(value.len() - 1);
    let value = value.parse::<u64>().ok()?;
    let multiplier = match suffix.to_lowercase().as_str() {
        "k" => 1024,
        "m" => 1024 * 1024,
        "g" => 1024 * 1024 * 1024,
        _ => 1,
    };
    Some(value * multiplier)
}

fn print_stats(files: &[VideoFile]) {
    let total_size: u64 = files.iter().map(|f| f.file_size).sum();
    let total_files = files.len();

    println!("Total files: {}", total_files);
    println!("Total size: {}", total_size.human_count_bytes());

    let codec_distribution =
        files
            .iter()
            .map(|f| f.codec.as_str())
            .fold(HashMap::new(), |mut acc, codec| {
                *acc.entry(codec).or_insert(0) += 1;
                acc
            });
    println!("File counts by codec:");
    for (codec, count) in codec_distribution {
        println!("\t{}: {}", codec, count);
    }
    let total_duration = files.iter().map(|f| f.duration).sum::<f64>();
    println!("Total duration: {}", total_duration.human_duration());
}

fn main() -> Result<()> {
    use std::env;

    let start = Instant::now();
    let args = Args::parse();

    if let Some(level) = args.log {
        env::set_var("RUST_LOG", level.to_string());
    }

    let database = Database::new()?;

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();
    color_eyre::install()?;

    match args.command {
        Command::Scan {
            exclude,
            min_size,
            path,
        } => {
            let min_size = min_size.as_deref().and_then(parse_bytes);
            let collector = Collector::new(database.clone(), path, exclude, min_size);
            collector.gather_files()?;
        }
        Command::Transcode {
            crf,
            effort,
            dry_run,
            replace,
            gpu,
            parallel,
            number,
        } => {
            let files = database.list_limit(number)?;
            let transcode_options = TranscodeOptions {
                crf,
                effort,
                dry_run,
                replace,
                gpu,
                parallel,
                progress_hidden: args.log.is_some(),
                ignored_codecs: vec!["av1".into(), "hevc".into()],
            };
            let files: Vec<_> = files.into_iter().map(From::from).collect();
            let transcoder = Transcoder::new(database, transcode_options, files);
            transcoder.transcode_all()?;
            let duration = start.elapsed();
            info!("total duration: {}", duration.human_duration());
        }
        Command::Info => {}
    }
    Ok(())
}
