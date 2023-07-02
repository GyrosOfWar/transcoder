use std::collections::HashMap;

use camino::Utf8PathBuf;
use clap::Parser;
use database::VideoFile;
use tracing_indicatif::IndicatifLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::database::Database;

mod collect;
mod database;
mod ffprobe;
mod transcode;

pub type Result<T> = std::result::Result<T, color_eyre::Report>;

#[derive(Parser, Debug)]
pub struct Args {
    /// Exclude files that contain this string
    #[clap(short, long)]
    pub exclude: Vec<String>,

    /// CRF value to use for encoding
    #[clap(short, long, default_value = "23")]
    pub crf: u8,

    /// Effort level to use for encoding
    #[clap(short, long, default_value = "4")]
    pub effort: u8,

    /// Codecs to transcode
    #[clap(short, long, default_value = "h264")]
    pub codecs: Vec<String>,

    /// Verbose output
    #[clap(short, long)]
    pub verbose: bool,

    /// The path to scan for video files
    pub path: Utf8PathBuf,
}

fn print_stats(files: &[VideoFile]) {
    let total_size: u64 = files.iter().map(|f| f.file_size).sum();
    let total_files = files.len();

    println!("Total files: {}", total_files);
    println!("Total size: {}", total_size);

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
        println!("{}: {}", codec, count);
    }
}

fn main() -> Result<()> {
    use std::env;
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }

    let indicatif_layer = IndicatifLayer::new();

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(indicatif_layer.get_stderr_writer()))
        .with(indicatif_layer)
        .init();
    color_eyre::install()?;

    let args = Args::parse();
    let database = Database::new()?;
    database.create_tables()?;

    let files = collect::gather_files(&args.path, args.exclude.clone())?;
    let files = collect::probe_files(files);
    if args.verbose {
        print_stats(&files);
    }

    let transcode_options = args.into();
    transcode::transcode_all(files, transcode_options)?;

    Ok(())
}
