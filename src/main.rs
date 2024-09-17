use std::collections::HashMap;
use std::time::Instant;

use camino::Utf8PathBuf;
use clap::Parser;
use collect::{FileSortOrder, VideoFile};
use human_repr::{HumanCount, HumanDuration};
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use crate::collect::Collector;
use crate::transcode::Transcoder;

mod collect;
mod ffprobe;
mod transcode;

pub type Result<T> = std::result::Result<T, color_eyre::Report>;

#[derive(Parser, Debug)]
pub struct Args {
    /// Exclude files that contain this string
    #[clap(short = 'E', long)]
    pub exclude: Vec<String>,

    /// CRF value to use for encoding
    #[clap(short, long, default_value = "24")]
    pub crf: u8,

    /// Effort level to use for encoding
    #[clap(short, long, default_value = "7")]
    pub effort: u8,

    /// Dry run, don't do anything
    #[clap(short, long)]
    pub dry_run: bool,

    /// Minimum file size to transcode
    #[clap(long)]
    pub min_size: Option<String>,

    /// Set the log level
    #[clap(short, long)]
    pub log: Option<tracing::level_filters::LevelFilter>,

    /// Replace the original file with the transcoded one
    #[clap(short, long)]
    pub replace: bool,

    /// Don't transcode, just print stats about the files at the location.
    #[clap(long)]
    pub stats: bool,

    /// Use the GPU (nvenv) for transcoding
    #[clap(long)]
    pub gpu: bool,

    /// Sort order in which the files should be processed
    #[clap(long)]
    pub sort: Option<FileSortOrder>,

    /// Number of files to process in parallel.
    #[clap(short, long)]
    pub parallel: u32,

    /// Limit how many files to process
    #[clap(short, long)]
    pub number: Option<usize>,

    /// The path to scan for video files
    pub path: Utf8PathBuf,
}

impl Args {
    pub fn min_size(&self) -> Option<u64> {
        self.min_size.as_ref().and_then(|s| parse_bytes(s))
    }
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

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();
    color_eyre::install()?;

    let collector = Collector::new(
        args.path.clone(),
        args.exclude.clone(),
        args.min_size(),
        args.sort,
        args.number,
    );
    let files = collector.gather_files()?;
    let files = collector.probe_files(files)?;

    if args.stats {
        print_stats(&files);
        return Ok(());
    }

    let transcode_options = args.into();
    let transcoder = Transcoder::new(transcode_options, files);
    transcoder.transcode_all()?;
    let duration = start.elapsed();
    info!("total duration: {}", duration.human_duration());

    Ok(())
}
