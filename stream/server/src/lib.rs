//! DCL Stream server. One renter at a time: a newly authenticated connection
//! (the renter's network changed, the app restarted) replaces the old one;
//! anything that fails the token proof is closed without a single video byte.

pub mod backend;

use anyhow::{anyhow, bail, Context};
use backend::{Backend, EncodedFrame, InputSink, VideoPipeline};
use bytes::Bytes;
use dcl_stream_proto::auth::{self, Token};
use dcl_stream_proto::fec::{encode_frame, FecParams, FrameMeta};
use dcl_stream_proto::msg::{ClientMsg, InputDatagram, InputEvent, ServerMsg, StreamSettings};
use dcl_stream_proto::net::{self, ControlStream};
use dcl_stream_proto::pacing::Pacer;
use dcl_stream_proto::rate::{KeyframeLimiter, RateConfig, RateController};
use dcl_stream_proto::wire::{flags, Kind};
use dcl_stream_proto::now_us;
use quinn::{Connection, Endpoint};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

pub struct ServerConfig {
    pub listen: SocketAddr,
    pub token: Token,
    /// Tier ceiling; the client may ask for less, never more.
    pub max_bitrate_kbps: u32,
}

pub struct Server {
    endpoint: Endpoint,
    fingerprint: [u8; 32],
    token: Token,
    max_bitrate_kbps: u32,
    backend: Arc<dyn Backend>,
    active: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

impl Server {
    pub fn bind(cfg: ServerConfig, backend: Arc<dyn Backend>) -> anyhow::Result<Server> {
        let (cert, key) = net::cert_from_token(&cfg.token)?;
        let fingerprint = auth::fingerprint(cert.as_ref());
        let endpoint = Endpoint::server(net::server_config(cert, key)?, cfg.listen).with_context(|| format!("bind {}", cfg.listen))?;
        Ok(Server {
            endpoint,
            fingerprint,
            token: cfg.token,
            max_bitrate_kbps: cfg.max_bitrate_kbps,
            backend,
            active: Arc::new(Mutex::new(None)),
        })
    }

    pub fn local_addr(&self) -> anyhow::Result<SocketAddr> {
        Ok(self.endpoint.local_addr()?)
    }

    /// SHA-256 of the certificate. Clients derive the same value from the
    /// token (`net::fingerprint_for_token`).
    pub fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let shared = Arc::new(self);
        while let Some(incoming) = shared.endpoint.accept().await {
            let s = shared.clone();
            tokio::spawn(async move {
                let remote = incoming.remote_address();
                match incoming.await {
                    Ok(conn) => {
                        if let Err(e) = s.handle(conn).await {
                            info!(%remote, "session ended: {e:#}");
                        }
                    }
                    Err(e) => debug!(%remote, "handshake failed: {e}"),
                }
            });
        }
        Ok(())
    }

    async fn handle(&self, conn: Connection) -> anyhow::Result<()> {
        let caps = self.backend.caps();
        let ctl = match net::server_handshake(&conn, &self.token, caps.clone()).await {
            Ok(c) => c,
            Err(e) => {
                conn.close(1u32.into(), b"auth");
                bail!("refused {}: {e}", conn.remote_address());
            }
        };
        let (stop_tx, stop_rx) = oneshot::channel();
        if let Some(old) = self.active.lock().unwrap().replace(stop_tx) {
            let _ = old.send(());
            info!("a new authenticated client replaces the old one");
        }
        info!(remote = %conn.remote_address(), "client authenticated");
        let result = Session { conn: conn.clone(), backend: self.backend.clone(), caps, max_bitrate_kbps: self.max_bitrate_kbps }
            .run(ctl, stop_rx)
            .await;
        conn.close(0u32.into(), b"end");
        result
    }
}

enum VideoCmd {
    Lost { first: u32, last: u32 },
    Keyframe,
    Bitrate(u32),
}

struct Session {
    conn: Connection,
    backend: Arc<dyn Backend>,
    caps: dcl_stream_proto::msg::Capabilities,
    max_bitrate_kbps: u32,
}

impl Session {
    async fn run(self, mut ctl: ControlStream, stop_rx: oneshot::Receiver<()>) -> anyhow::Result<()> {
        let settings = match tokio::time::timeout(Duration::from_secs(5), net::recv_msg::<ClientMsg>(&mut ctl.recv)).await {
            Ok(Ok(Some(ClientMsg::Settings(s)))) => s,
            _ => bail!("client did not send Settings"),
        };
        let s: StreamSettings = settings.clamp_to(&self.caps, self.max_bitrate_kbps);
        let Some(&codec) = s.codecs.first() else {
            net::send_msg(&mut ctl.send, &ServerMsg::End { reason: "no common codec".into() }).await?;
            bail!("no common codec");
        };
        let mut rate = RateController::new(RateConfig::new(s.max_bitrate_kbps));
        let d = rate.current();
        let pipeline = self.backend.open_video(&s, codec, d.video_kbps)?;
        let vinfo = pipeline.info();
        let max_dgram = self.conn.max_datagram_size().ok_or_else(|| anyhow!("peer does not accept datagrams"))?;
        let shard_bytes = FecParams::shard_bytes_for_datagram(max_dgram);
        if shard_bytes < 256 {
            bail!("datagrams too small ({max_dgram} bytes)");
        }
        let audio_pipe = if s.audio { self.backend.open_audio()? } else { None };
        net::send_msg(
            &mut ctl.send,
            &ServerMsg::Start {
                codec,
                width: vinfo.width,
                height: vinfo.height,
                fps: vinfo.fps,
                shard_bytes: shard_bytes as u16,
                bitrate_kbps: d.video_kbps,
                audio: audio_pipe.is_some(),
            },
        )
        .await?;
        info!(?codec, w = vinfo.width, h = vinfo.height, fps = vinfo.fps, encoder = %vinfo.encoder, shard_bytes, "streaming");

        let stop = Arc::new(AtomicBool::new(false));
        let fec_pct = Arc::new(AtomicU8::new(d.fec_pct));
        let total_kbps = Arc::new(AtomicU32::new(d.total_kbps));
        let (cmd_tx, cmd_rx) = std_mpsc::channel::<VideoCmd>();
        let (frame_tx, mut frame_rx) = mpsc::channel::<(EncodedFrame, u8)>(4);

        // Capture + encode thread.
        let video_thread = {
            let stop = stop.clone();
            let start_kbps = d.video_kbps;
            std::thread::Builder::new().name("video".into()).spawn(move || video_driver(pipeline, start_kbps, cmd_rx, frame_tx, stop))?
        };

        // Video send task.
        let send_conn = self.conn.clone();
        let (fp, tk) = (fec_pct.clone(), total_kbps.clone());
        let mut video_send = tokio::spawn(async move {
            let mut pacer = Pacer::new(tk.load(Ordering::Relaxed), 2.0, shard_bytes * 16);
            let mut id: u32 = 0;
            while let Some((frame, fl)) = frame_rx.recv().await {
                let mut params = FecParams::new(shard_bytes, fp.load(Ordering::Relaxed));
                // Small frames: 2 parity shards cover any 2 losses in the group.
                params.min_parity = 2;
                pacer.set_rate(tk.load(Ordering::Relaxed), 2.0);
                let meta = FrameMeta { kind: Kind::Video, flags: fl, frame_id: id, capture_us: frame.capture_us };
                id = id.wrapping_add(1);
                let dgrams = match encode_frame(&frame.data, meta, &params) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!("frame dropped: {e}");
                        continue;
                    }
                };
                for d in dgrams {
                    let wait = pacer.delay_us(d.len(), now_us());
                    if wait >= 1_000 {
                        tokio::time::sleep(Duration::from_micros(wait)).await;
                    }
                    match send_conn.send_datagram(Bytes::from(d)) {
                        Ok(()) => {}
                        Err(quinn::SendDatagramError::TooLarge) => {
                            warn!("shard larger than the path allows; frame cut short");
                            break;
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
            }
            anyhow::Ok(())
        });

        // Audio thread + send task.
        let mut audio_tasks = None;
        if let Some(mut ap) = audio_pipe {
            let (atx, mut arx) = mpsc::channel(32);
            let st = stop.clone();
            std::thread::Builder::new().name("audio".into()).spawn(move || {
                while !st.load(Ordering::Relaxed) {
                    match ap.next_packet() {
                        Ok(p) => {
                            if atx.blocking_send(p).is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            warn!("audio stopped: {e:#}");
                            break;
                        }
                    }
                }
            })?;
            let c = self.conn.clone();
            audio_tasks = Some(tokio::spawn(async move {
                let mut id: u32 = 0;
                while let Some(p) = arx.recv().await {
                    // One packet per frame, duplicated by one parity shard: Opus packets are tiny.
                    let shard = (p.data.len() + 1) & !1;
                    let meta = FrameMeta { kind: Kind::Audio, flags: 0, frame_id: id, capture_us: p.capture_us };
                    id = id.wrapping_add(1);
                    if let Ok(dgrams) = encode_frame(&p.data, meta, &FecParams { shard_bytes: shard.max(2), parity_pct: 100, min_parity: 1, group_data_shards: 1 }) {
                        for d in dgrams {
                            c.send_datagram(Bytes::from(d))?;
                        }
                    }
                }
                anyhow::Ok(())
            }));
        }

        // Input: ordered events on a uni stream, motion as datagrams.
        let sink: Arc<Mutex<Box<dyn InputSink>>> = Arc::new(Mutex::new(self.backend.open_input()?));
        let input_conn = self.conn.clone();
        let sink_ev = sink.clone();
        let input_stream = tokio::spawn(async move {
            while let Ok(mut uni) = input_conn.accept_uni().await {
                let sink = sink_ev.clone();
                tokio::spawn(async move {
                    while let Ok(Some(e)) = net::recv_msg::<InputEvent>(&mut uni).await {
                        if let Err(err) = sink.lock().unwrap().event(e) {
                            warn!("input: {err:#}");
                        }
                    }
                });
            }
        });
        let dg_conn = self.conn.clone();
        let input_dgrams = tokio::spawn(async move {
            let mut last_seq: Option<u32> = None;
            while let Ok(d) = dg_conn.read_datagram().await {
                let Ok(m) = InputDatagram::from_datagram(&d) else { continue };
                // Latest wins: drop anything older than what we already applied.
                if last_seq.is_some_and(|l| m.seq().wrapping_sub(l) as i32 <= 0) {
                    continue;
                }
                last_seq = Some(m.seq());
                if let Err(err) = sink.lock().unwrap().motion(m) {
                    warn!("input: {err:#}");
                }
            }
        });

        // Control: reports, losses, pings.
        let control = async {
            let mut last_rate = rate.current();
            loop {
                let Some(m) = net::recv_msg::<ClientMsg>(&mut ctl.recv).await? else { return anyhow::Ok("client closed the control stream") };
                match m {
                    ClientMsg::FrameLost { first, last } => {
                        let _ = cmd_tx.send(VideoCmd::Lost { first, last });
                    }
                    ClientMsg::RequestKeyframe => {
                        let _ = cmd_tx.send(VideoCmd::Keyframe);
                    }
                    ClientMsg::Report(r) => {
                        let d = rate.on_report(&r, now_us());
                        fec_pct.store(d.fec_pct, Ordering::Relaxed);
                        total_kbps.store(d.total_kbps, Ordering::Relaxed);
                        if d != last_rate {
                            let _ = cmd_tx.send(VideoCmd::Bitrate(d.video_kbps));
                            net::send_msg(&mut ctl.send, &ServerMsg::Rate { video_kbps: d.video_kbps, fec_pct: d.fec_pct }).await?;
                            last_rate = d;
                        }
                    }
                    ClientMsg::Ping { client_us } => {
                        net::send_msg(&mut ctl.send, &ServerMsg::Pong { client_us, server_us: now_us() }).await?;
                    }
                    ClientMsg::End => return Ok("client ended the session"),
                    ClientMsg::Open { .. } | ClientMsg::Auth { .. } | ClientMsg::Settings(_) => {}
                }
            }
        };

        // Audio failing turns audio off; it never ends the session.
        let audio_wait = async {
            if let Some(t) = audio_tasks.as_mut() {
                match t.await {
                    Ok(Err(e)) => warn!("audio send stopped: {e:#}"),
                    Err(e) => warn!("audio task: {e}"),
                    Ok(Ok(())) => info!("audio ended; video goes on"),
                }
            }
            std::future::pending::<()>().await
        };
        let outcome: anyhow::Result<&str> = tokio::select! {
            r = control => r,
            r = &mut video_send => match r { Ok(Ok(())) => Ok("video ended"), Ok(Err(e)) => Err(e), Err(e) => Err(e.into()) },
            _ = audio_wait => unreachable!("audio_wait never completes"),
            _ = stop_rx => Ok("replaced by a new client"),
            e = self.conn.closed() => Ok(match e { quinn::ConnectionError::ApplicationClosed(_) => "client closed", _ => "connection lost" }),
        };
        stop.store(true, Ordering::Relaxed);
        video_send.abort();
        input_stream.abort();
        input_dgrams.abort();
        if let Some(t) = audio_tasks {
            t.abort();
        }
        // Never block a runtime worker on the thread: the task that must drop
        // the frame channel (and so release the thread) may be queued on it.
        let _ = tokio::task::spawn_blocking(move || video_thread.join()).await;
        let why = outcome?;
        let _ = net::send_msg(&mut ctl.send, &ServerMsg::End { reason: why.into() }).await;
        info!("session over: {why}");
        Ok(())
    }
}

/// Owns the pipeline on its thread. Applies keyframe limits, reference-frame
/// invalidation and bitrate changes between frames.
fn video_driver(
    mut p: Box<dyn VideoPipeline>,
    start_kbps: u32,
    cmds: std_mpsc::Receiver<VideoCmd>,
    out: mpsc::Sender<(EncodedFrame, u8)>,
    stop: Arc<AtomicBool>,
) {
    let rfi = p.info().rfi;
    let mut limiter = KeyframeLimiter::new(200_000);
    // RFI needs a good reference before the loss. A keyframe empties the
    // encoder's reference list, so a loss that covers the latest keyframe
    // can only be repaired with a new one.
    let mut index: u64 = 0;
    let mut last_key: Option<u64> = None;
    let mut recovery_next = false;
    let mut current_kbps: Option<u32> = Some(start_kbps);
    let mut last_bitrate_change_us = 0u64;
    let mut pending_kbps: Option<u32> = None;
    while !stop.load(Ordering::Relaxed) {
        let now = now_us();
        while let Ok(c) = cmds.try_recv() {
            match c {
                VideoCmd::Lost { first, last } => {
                    let has_reference = last_key.is_some_and(|k| (first as u64) > k);
                    if rfi && has_reference && last.wrapping_sub(first) < 16 && p.invalidate(first as u64, last as u64) {
                        recovery_next = true;
                    } else if limiter.request(now) {
                        p.force_keyframe();
                    }
                }
                VideoCmd::Keyframe => {
                    if limiter.request(now) {
                        p.force_keyframe();
                    }
                }
                VideoCmd::Bitrate(k) => pending_kbps = Some(k),
            }
        }
        if limiter.poll(now) {
            p.force_keyframe();
        }
        // Bitrate: only for a 10 % change, at most every 2 s (an FFmpeg
        // encoder reopen costs a keyframe).
        if let Some(k) = pending_kbps {
            let big = current_kbps.is_none_or(|c| (k as f64 - c as f64).abs() >= c as f64 * 0.10);
            let cut = current_kbps.is_some_and(|c| k < c);
            if big && (cut || now.saturating_sub(last_bitrate_change_us) >= 2_000_000) {
                match p.set_bitrate(k) {
                    Ok(()) => {
                        current_kbps = Some(k);
                        last_bitrate_change_us = now;
                    }
                    Err(e) => warn!("bitrate change failed: {e:#}"),
                }
                pending_kbps = None;
            }
        }
        let frame = match p.next_frame() {
            Ok(f) => f,
            Err(e) => {
                warn!("video stopped: {e:#}");
                break;
            }
        };
        let mut fl = 0;
        let this = index;
        index += 1;
        if frame.keyframe {
            last_key = Some(this);
            fl |= flags::KEYFRAME;
            limiter.sent(now_us());
            recovery_next = false;
        } else if std::mem::take(&mut recovery_next) {
            fl |= flags::RECOVERY;
        }
        if out.blocking_send((frame, fl)).is_err() {
            break;
        }
    }
}
