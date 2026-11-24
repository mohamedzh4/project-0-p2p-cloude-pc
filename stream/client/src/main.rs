//! dcl-stream-client: plays a Pro session.
//!
//! The website gives the renter a launch link with the guest address,
//! certificate fingerprint and session token; `--headless` runs without a
//! window and prints stats (network tests, CI).

use anyhow::Context;
use clap::Parser;
use dcl_stream_client::{connect, ClientEvent, ConnectParams};
use dcl_stream_proto::auth;
use dcl_stream_proto::msg::{Codec, StreamSettings};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(version, about = "DCL Stream client")]
struct Args {
    /// Guest address on the mesh, e.g. 100.64.0.7:47100
    #[arg(long, env = "DCL_STREAM_ADDR")]
    addr: SocketAddr,
    /// Server certificate SHA-256 (hex). Default: derived from the token.
    #[arg(long, env = "DCL_STREAM_FINGERPRINT")]
    fingerprint: Option<String>,
    /// Session token (64 hex characters), from the Matchmaker.
    #[arg(long, env = "DCL_STREAM_TOKEN")]
    token: String,
    #[arg(long, default_value_t = 1920)]
    width: u32,
    #[arg(long, default_value_t = 1080)]
    height: u32,
    #[arg(long, default_value_t = 60)]
    fps: u16,
    #[arg(long, default_value_t = 40_000)]
    max_bitrate_kbps: u32,
    /// Codecs, best first.
    #[arg(long, value_delimiter = ',', default_value = "hevc,h264")]
    codecs: Vec<String>,
    #[arg(long)]
    no_audio: bool,
    /// No window: receive for --seconds and print stats as JSON.
    #[arg(long)]
    headless: bool,
    #[arg(long, default_value_t = 10)]
    seconds: u64,
}

fn parse_codec(s: &str) -> anyhow::Result<Codec> {
    match s.trim().to_ascii_lowercase().as_str() {
        "h264" | "avc" => Ok(Codec::H264),
        "hevc" | "h265" => Ok(Codec::Hevc),
        "av1" => Ok(Codec::Av1),
        o => anyhow::bail!("unknown codec {o}"),
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let a = Args::parse();
    let token = auth::parse_token(&a.token).context("token")?;
    let fingerprint = match &a.fingerprint {
        Some(f) => auth::parse_fingerprint(f)?,
        None => dcl_stream_proto::net::fingerprint_for_token(&token)?,
    };
    let params = ConnectParams {
        addr: a.addr,
        fingerprint,
        token,
        settings: StreamSettings {
            width: a.width,
            height: a.height,
            fps: a.fps,
            codecs: a.codecs.iter().map(|c| parse_codec(c)).collect::<anyhow::Result<_>>()?,
            max_bitrate_kbps: a.max_bitrate_kbps,
            audio: !a.no_audio,
        },
    };
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    if a.headless {
        return rt.block_on(headless(params, Duration::from_secs(a.seconds)));
    }
    run_player(rt, params)
}

async fn headless(params: ConnectParams, dur: Duration) -> anyhow::Result<()> {
    let mut s = connect(params).await?;
    eprintln!("streaming {:?} {}x{}@{} from {}", s.start.codec, s.start.width, s.start.height, s.start.fps, s.start.caps.encoder);
    let end = Instant::now() + dur;
    let mut why = String::from("time up");
    while let Ok(Some(ev)) = tokio::time::timeout_at(end.into(), s.events.recv()).await {
        if let ClientEvent::Ended(r) = ev {
            why = r;
            break;
        }
    }
    let stats = s.stats.lock().unwrap().clone();
    println!("{}", serde_json::to_string_pretty(&serde_json::json!({ "ended": why, "stats": stats }))?);
    s.close().await;
    Ok(())
}

#[cfg(feature = "player")]
fn run_player(rt: tokio::runtime::Runtime, params: ConnectParams) -> anyhow::Result<()> {
    dcl_stream_client::player::run(rt, params)
}

#[cfg(not(feature = "player"))]
fn run_player(_rt: tokio::runtime::Runtime, _params: ConnectParams) -> anyhow::Result<()> {
    anyhow::bail!("this build has no window; rebuild with --features player or use --headless")
}
