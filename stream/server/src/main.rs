//! dcl-stream-server: runs in the Pro guest after dcl-guest-join.
//!
//! Reads the session token from a file (written by the join script from the
//! Matchmaker bootstrap), makes a fresh certificate, writes its fingerprint
//! where the join script reports it to the Matchmaker, then serves one renter.

use anyhow::Context;
use clap::{Parser, ValueEnum};
use dcl_stream_proto::auth;
use dcl_stream_server::backend::{synthetic::Synthetic, Backend};
use dcl_stream_server::{Server, ServerConfig};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Source {
    /// Test frames, no capture device (network-path tests).
    Synthetic,
    /// FFmpeg capture and hardware encode (needs the `ffmpeg` build feature).
    Ffmpeg,
}

#[derive(Parser, Debug)]
#[command(version, about = "DCL Stream server (Pro guest)")]
struct Args {
    /// Address to listen on. Use the guest's mesh IP so nothing else can reach it.
    #[arg(long, env = "DCL_STREAM_LISTEN", default_value = "[::]:47100")]
    listen: SocketAddr,
    /// File holding the 64-hex-character session token.
    #[arg(long, env = "DCL_STREAM_TOKEN_FILE", default_value = "/run/dcl/stream-token")]
    token_file: PathBuf,
    /// Where to write the certificate fingerprint (hex; clients can also derive it from the token).
    #[arg(long, env = "DCL_STREAM_FINGERPRINT_FILE", default_value = "/run/dcl/stream-fingerprint")]
    fingerprint_file: PathBuf,
    #[arg(long, env = "DCL_STREAM_MAX_KBPS", default_value_t = 80_000)]
    max_bitrate_kbps: u32,
    #[arg(long, value_enum, env = "DCL_STREAM_SOURCE", default_value = "ffmpeg")]
    source: Source,
    /// X display (ffmpeg source).
    #[arg(long, env = "DISPLAY", default_value = ":0")]
    display: String,
    /// PulseAudio source for game audio; empty turns audio off.
    #[arg(long, env = "DCL_STREAM_AUDIO", default_value = "default")]
    audio: String,
    /// Encoders to try, in order (ffmpeg names).
    #[arg(long, env = "DCL_STREAM_ENCODERS", value_delimiter = ',', default_value = "nvenc,vaapi,software")]
    encoders: Vec<String>,
    /// Virtual input devices through /dev/uinput.
    #[arg(long, env = "DCL_STREAM_UINPUT", default_value_t = true, action = clap::ArgAction::Set)]
    uinput: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let a = Args::parse();
    let token_hex = std::fs::read_to_string(&a.token_file).with_context(|| format!("read {}", a.token_file.display()))?;
    let token = auth::parse_token(&token_hex)?;
    let backend = make_backend(&a)?;
    let server = Server::bind(ServerConfig { listen: a.listen, token, max_bitrate_kbps: a.max_bitrate_kbps }, backend)?;
    let fp = hex::encode(server.fingerprint());
    write_private(&a.fingerprint_file, &fp)?;
    tracing::info!(addr = %server.local_addr()?, fingerprint = %fp, "listening");
    tokio::select! {
        r = server.run() => r,
        _ = tokio::signal::ctrl_c() => Ok(()),
    }
}

fn write_private(path: &std::path::Path, text: &str) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

fn make_backend(a: &Args) -> anyhow::Result<Arc<dyn Backend>> {
    match a.source {
        Source::Synthetic => Ok(Arc::new(Synthetic { audio: true, ..Default::default() })),
        Source::Ffmpeg => ffmpeg_backend(a),
    }
}

#[cfg(feature = "ffmpeg")]
fn ffmpeg_backend(a: &Args) -> anyhow::Result<Arc<dyn Backend>> {
    use dcl_stream_server::backend::ffmpeg::{Capture, FfmpegBackend};
    let capture = Capture::X11 { display: a.display.clone() };
    let audio = (!a.audio.is_empty()).then(|| a.audio.clone());
    Ok(Arc::new(FfmpegBackend::new(capture, audio, a.encoders.clone(), a.uinput)?))
}

#[cfg(not(feature = "ffmpeg"))]
fn ffmpeg_backend(_a: &Args) -> anyhow::Result<Arc<dyn Backend>> {
    anyhow::bail!("this build has no FFmpeg support; rebuild with --features ffmpeg or use --source synthetic")
}
