//! End-to-end: the real server and client over real QUIC on localhost, with
//! the synthetic backend and an optional lossy UDP proxy in between.

use dcl_stream_client::{connect, ClientEvent, ConnectParams, Session};
use dcl_stream_proto::auth::{self, Token};
use dcl_stream_proto::msg::{Codec, InputEvent, MouseButton, StreamSettings};
use dcl_stream_proto::wire::flags;
use dcl_stream_server::backend::synthetic::{verify_frame, Synthetic};
use dcl_stream_server::{Server, ServerConfig};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

struct Running {
    addr: SocketAddr,
    fingerprint: [u8; 32],
    token: Token,
}

fn start_server(backend: Synthetic) -> Running {
    let token = auth::new_token();
    let server = Server::bind(
        ServerConfig { listen: "127.0.0.1:0".parse().unwrap(), token, max_bitrate_kbps: 20_000 },
        Arc::new(backend),
    )
    .unwrap();
    let r = Running { addr: server.local_addr().unwrap(), fingerprint: server.fingerprint(), token };
    assert_eq!(r.fingerprint, dcl_stream_proto::net::fingerprint_for_token(&token).unwrap());
    tokio::spawn(server.run());
    r
}

fn settings() -> StreamSettings {
    StreamSettings { width: 1280, height: 720, fps: 60, codecs: vec![Codec::Hevc, Codec::H264], max_bitrate_kbps: 10_000, audio: true }
}

fn params(r: &Running, addr: SocketAddr) -> ConnectParams {
    ConnectParams { addr, fingerprint: r.fingerprint, token: r.token, settings: settings() }
}

/// UDP relay that drops server->client packets. `drop(n)` decides for the
/// n-th packet; the first 40 always pass so the handshake is not the test.
async fn lossy_proxy(server: SocketAddr, drop: impl Fn(u64) -> bool + Send + Sync + 'static) -> (SocketAddr, Arc<AtomicU64>) {
    let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
    let addr = sock.local_addr().unwrap();
    let dropped = Arc::new(AtomicU64::new(0));
    let d2 = dropped.clone();
    tokio::spawn(async move {
        let mut client: Option<SocketAddr> = None;
        let mut buf = vec![0u8; 65536];
        let mut n: u64 = 0;
        while let Ok((len, from)) = sock.recv_from(&mut buf).await {
            if from == server {
                n += 1;
                if n > 40 && drop(n) {
                    d2.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                if let Some(c) = client {
                    let _ = sock.send_to(&buf[..len], c).await;
                }
            } else {
                client = Some(from);
                let _ = sock.send_to(&buf[..len], server).await;
            }
        }
    });
    (addr, dropped)
}

/// Deterministic "random" in [0, 1) from a packet number.
fn rnd(n: u64) -> f64 {
    let mut x = n.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    (x >> 11) as f64 / (1u64 << 53) as f64
}

struct Seen {
    video: Vec<(u32, u8)>,
    audio: u64,
    bad: u64,
    ended: Option<String>,
}

async fn watch(s: &mut Session, dur: Duration) -> Seen {
    let mut seen = Seen { video: vec![], audio: 0, bad: 0, ended: None };
    let end = Instant::now() + dur;
    while let Ok(Some(ev)) = tokio::time::timeout_at(end.into(), s.events.recv()).await {
        match ev {
            ClientEvent::Video(f) => {
                if verify_frame(&f.data) != Some(f.frame_id as u64) {
                    seen.bad += 1;
                }
                seen.video.push((f.frame_id, f.flags));
            }
            ClientEvent::Audio(f) => {
                if verify_frame(&f.data).is_none() {
                    seen.bad += 1;
                }
                seen.audio += 1;
            }
            ClientEvent::Ended(r) => {
                seen.ended = Some(r);
                break;
            }
            ClientEvent::Rumble { .. } => {}
        }
    }
    seen
}

/// Waits for the first video frame; returns how many bad frames it saw.
async fn watch_until_first_frame(s: &mut Session) -> u64 {
    let end = Instant::now() + Duration::from_secs(5);
    while let Ok(Some(ev)) = tokio::time::timeout_at(end.into(), s.events.recv()).await {
        if let ClientEvent::Video(f) = ev {
            return (verify_frame(&f.data) != Some(f.frame_id as u64)) as u64;
        }
    }
    panic!("no video frame within 5 s");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn clean_stream_delivers_every_frame_intact_and_in_order() {
    let r = start_server(Synthetic { audio: true, ..Default::default() });
    let mut s = connect(params(&r, r.addr)).await.unwrap();
    assert_eq!(s.start.codec, Codec::Hevc);
    assert_eq!((s.start.width, s.start.height, s.start.fps), (1280, 720, 60));
    let seen = watch(&mut s, Duration::from_secs(2)).await;
    assert_eq!(seen.bad, 0);
    assert!(seen.video.len() >= 90, "only {} frames in 2 s at 60 fps", seen.video.len());
    assert!(seen.video.windows(2).all(|w| w[1].0 == w[0].0 + 1), "frames out of order or missing");
    assert!(seen.video[0].1 & flags::KEYFRAME != 0, "stream starts with a keyframe");
    assert!(seen.audio >= 300, "only {} audio packets in 2 s at 5 ms", seen.audio);
    let st = s.stats.lock().unwrap().clone();
    assert_eq!(st.frames_lost, 0);
    assert!(st.rtt_us.is_some(), "ping/pong measured the RTT");
    s.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_token_is_refused_before_any_video() {
    let r = start_server(Synthetic::default());
    let mut p = params(&r, r.addr);
    p.token = auth::new_token();
    let err = connect(p).await.err().expect("must be refused");
    assert!(format!("{err:#}").contains("auth") || format!("{err:#}").contains("closed"), "{err:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_fingerprint_is_refused_by_the_client() {
    let r = start_server(Synthetic::default());
    let mut p = params(&r, r.addr);
    p.fingerprint = [0xAA; 32];
    let err = connect(p).await.err().expect("must be refused");
    assert!(format!("{err:#}").contains("fingerprint"), "{err:#}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_client_replaces_the_first() {
    let r = start_server(Synthetic::default());
    let mut a = connect(params(&r, r.addr)).await.unwrap();
    let first = watch(&mut a, Duration::from_millis(300)).await;
    assert!(!first.video.is_empty());
    let mut b = connect(params(&r, r.addr)).await.unwrap();
    let a_after = watch(&mut a, Duration::from_secs(2)).await;
    assert!(a_after.ended.is_some(), "the old client is told to stop");
    let b_seen = watch(&mut b, Duration::from_millis(500)).await;
    assert!(b_seen.video.len() > 10);
    assert_eq!(b_seen.bad, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_percent_random_loss_is_repaired_by_fec() {
    let r = start_server(Synthetic::default());
    let (proxy, dropped) = lossy_proxy(r.addr, |n| rnd(n) < 0.05).await;
    let mut s = connect(params(&r, proxy)).await.unwrap();
    // Count from the first frame: the proxy also drops QUIC setup packets,
    // and their resend timers can delay the start by a second.
    let first = watch_until_first_frame(&mut s).await;
    let mut seen = watch(&mut s, Duration::from_secs(3)).await;
    seen.bad += first;
    let st = s.stats.lock().unwrap().clone();
    assert!(dropped.load(Ordering::Relaxed) > 50, "the proxy dropped {} packets; {} frames; {st:?}", dropped.load(Ordering::Relaxed), seen.video.len());
    assert_eq!(seen.bad, 0, "every delivered frame is byte-exact");
    assert!(st.frames_recovered_by_fec > 20, "{st:?}");
    // FEC is the point: almost every frame survives 5 % loss.
    assert!(seen.video.len() >= 160, "{} frames shown, {st:?}", seen.video.len());
    assert!(st.frames_lost <= 3, "{st:?}");
    assert!(st.fec_pct >= 10, "FEC went up with loss: {st:?}");
}

async fn heavy_loss(rfi: bool) -> (Seen, dcl_stream_client::Stats, Vec<u32>) {
    let backend = Synthetic { rfi, ..Default::default() };
    let log = backend.bitrate_log.clone();
    let r = start_server(backend);
    // bursts: 30 % of packets in a 200-packet window out of every 1000,
    // after the first keyframe is through
    let (proxy, _) = lossy_proxy(r.addr, |n| n > 400 && (n % 1000) < 200 && rnd(n) < 0.3).await;
    let mut s = connect(params(&r, proxy)).await.unwrap();
    let seen = watch(&mut s, Duration::from_secs(4)).await;
    let st = s.stats.lock().unwrap().clone();
    let bl = log.lock().unwrap().clone();
    (seen, st, bl)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_unrecoverable_loss_rfi_resumes_the_stream_without_keyframes() {
    let (seen, st, bitrates) = heavy_loss(true).await;
    assert!(st.loss_events > 0, "{st:?}");
    assert_eq!(seen.bad, 0);
    assert!(seen.video.iter().any(|(_, f)| f & flags::RECOVERY != 0), "the server answered with RFI");
    assert_eq!(seen.video.iter().filter(|(_, f)| f & flags::KEYFRAME != 0).count(), 1, "only the first keyframe");
    assert!(seen.video.len() > 100, "the stream kept going: {} frames, {st:?}", seen.video.len());
    assert!(bitrates.iter().any(|&k| k < 10_000 * 3 / 4 * 100 / 120), "loss bursts cut the bitrate: {bitrates:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_unrecoverable_loss_without_rfi_a_keyframe_resumes_the_stream() {
    let (seen, st, _) = heavy_loss(false).await;
    assert!(st.loss_events > 0, "{st:?}");
    assert_eq!(seen.bad, 0);
    assert!(seen.video.iter().filter(|(_, f)| f & flags::KEYFRAME != 0).count() > 1, "{st:?}");
    assert!(seen.video.iter().all(|(_, f)| f & flags::RECOVERY == 0));
    assert!(seen.video.len() > 100, "{} frames, {st:?}", seen.video.len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn input_reaches_the_server_and_motion_is_batched() {
    let backend = Synthetic::default();
    let log = backend.input_log.clone();
    let r = start_server(backend);
    let s = connect(params(&r, r.addr)).await.unwrap();
    s.input.key(30, true);
    s.input.key(30, false);
    for _ in 0..100 {
        s.input.mouse_move(1, -2);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    s.input.button(MouseButton::Left, true);
    s.input.send(InputEvent::Scroll { dx: 0, dy: -120 });
    s.input.gamepad_axes(0, [100, -100, 0, 0, 255, 0], 0, 1);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let log = log.lock().unwrap().clone();
    assert!(log.contains(&"Key { code: 30, down: true }".to_string()), "{log:?}");
    assert!(log.contains(&"Key { code: 30, down: false }".to_string()));
    assert!(log.contains(&"MouseButton { button: Left, down: true }".to_string()));
    assert!(log.iter().any(|l| l.starts_with("GamepadAxes")));
    let (mut dx, mut dy, mut moves) = (0, 0, 0);
    for l in &log {
        if let Some(rest) = l.strip_prefix("MouseMove { seq: ") {
            moves += 1;
            let parts: Vec<i32> = rest.split(|c: char| !(c.is_ascii_digit() || c == '-')).filter_map(|x| x.parse().ok()).collect();
            dx += parts[1];
            dy += parts[2];
        }
    }
    assert_eq!((dx, dy), (100, -200), "{log:?}");
    assert!(moves < 100, "100 moves were batched into {moves} datagrams");
}
