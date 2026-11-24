//! DCL Stream client library: connect, authenticate, receive video and audio,
//! report the network, recover from loss, send input.
//!
//! The window (decode + display + keyboard/mouse/gamepad capture) is in
//! `player` behind the `player` feature; everything here is OS-neutral and
//! what the tests drive.

#[cfg(feature = "player")]
pub mod player;

use anyhow::{anyhow, bail, Context};
use bytes::Bytes;
use dcl_stream_proto::auth::Token;
use dcl_stream_proto::clock::{ClockSync, QueueDelay};
use dcl_stream_proto::msg::{Capabilities, ClientMsg, Codec, InputDatagram, InputEvent, MouseButton, NetworkReport, ServerMsg, StreamSettings};
use dcl_stream_proto::net::{self, SERVER_NAME};
use dcl_stream_proto::reassembly::{Assembler, AssemblerConfig, CompleteFrame, Event, RecoveryGate};
use dcl_stream_proto::wire::{Kind, ShardHeader};
use dcl_stream_proto::now_us;
use quinn::{Connection, Endpoint};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, warn};

pub const REPORT_INTERVAL: Duration = Duration::from_millis(50);
pub const PING_INTERVAL: Duration = Duration::from_secs(1);

pub struct ConnectParams {
    pub addr: SocketAddr,
    /// SHA-256 of the server certificate, from the Matchmaker.
    pub fingerprint: [u8; 32],
    pub token: Token,
    pub settings: StreamSettings,
}

#[derive(Clone, Debug)]
pub struct StartInfo {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u16,
    pub audio: bool,
    pub caps: Capabilities,
}

/// Running totals for the stats overlay and `--headless` output.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub frames_shown: u64,
    pub frames_lost: u64,
    pub frames_recovered_by_fec: u64,
    pub frames_dropped_waiting_for_recovery: u64,
    pub loss_events: u64,
    pub shards_received: u64,
    pub shards_missing: u64,
    pub audio_packets: u64,
    pub bytes_video: u64,
    pub rtt_us: Option<u64>,
    pub queue_delay_us: u32,
    /// Capture on the server to complete on this client (needs clock sync).
    pub last_latency_us: Option<u64>,
    pub video_kbps: u32,
    pub fec_pct: u8,
}

pub enum ClientEvent {
    Video(CompleteFrame),
    Audio(CompleteFrame),
    Rumble { pad: u8, low: u16, high: u16 },
    Ended(String),
}

pub struct Session {
    pub start: StartInfo,
    pub events: mpsc::Receiver<ClientEvent>,
    pub input: InputSender,
    pub stats: Arc<Mutex<Stats>>,
    conn: Connection,
    ctl_tx: mpsc::UnboundedSender<ClientMsg>,
    _endpoint: Endpoint,
}

/// Asks the server for a keyframe; cheap to clone, usable from any thread.
#[derive(Clone)]
pub struct KeyframeRequester(mpsc::UnboundedSender<ClientMsg>);

impl KeyframeRequester {
    pub fn request(&self) {
        let _ = self.0.send(ClientMsg::RequestKeyframe);
    }
}

impl Session {
    /// Asks the server for a keyframe (decoder reset, window resize).
    pub fn request_keyframe(&self) {
        let _ = self.ctl_tx.send(ClientMsg::RequestKeyframe);
    }

    pub fn keyframe_requester(&self) -> KeyframeRequester {
        KeyframeRequester(self.ctl_tx.clone())
    }

    pub async fn close(self) {
        let _ = self.ctl_tx.send(ClientMsg::End);
        tokio::time::sleep(Duration::from_millis(50)).await;
        self.conn.close(0u32.into(), b"bye");
    }
}

pub async fn connect(p: ConnectParams) -> anyhow::Result<Session> {
    let bind: SocketAddr = if p.addr.is_ipv6() { "[::]:0".parse()? } else { "0.0.0.0:0".parse()? };
    let mut endpoint = Endpoint::client(bind)?;
    endpoint.set_default_client_config(net::client_config(p.fingerprint)?);
    let conn = endpoint
        .connect(p.addr, SERVER_NAME)?
        .await
        .with_context(|| format!("connect to {} (wrong fingerprint or unreachable)", p.addr))?;
    let (mut ctl, caps) = net::client_handshake(&conn, &p.token).await?;
    net::send_msg(&mut ctl.send, &ClientMsg::Settings(p.settings.clone())).await?;
    let start = match tokio::time::timeout(Duration::from_secs(10), net::recv_msg::<ServerMsg>(&mut ctl.recv)).await {
        Ok(Ok(Some(ServerMsg::Start { codec, width, height, fps, audio, .. }))) => StartInfo { codec, width, height, fps, audio, caps },
        Ok(Ok(Some(ServerMsg::End { reason }))) => bail!("server refused: {reason}"),
        _ => bail!("no Start from server"),
    };

    let stats = Arc::new(Mutex::new(Stats::default()));
    let clock = Arc::new(Mutex::new(ClockSync::default()));
    let (ev_tx, ev_rx) = mpsc::channel(64);
    let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<ClientMsg>();

    // Control writer.
    let mut send = ctl.send;
    tokio::spawn(async move {
        while let Some(m) = ctl_rx.recv().await {
            let end = matches!(m, ClientMsg::End);
            if net::send_msg(&mut send, &m).await.is_err() || end {
                break;
            }
        }
        let _ = send.finish();
    });

    // Control reader.
    let mut recv = ctl.recv;
    let (st, ck, tx) = (stats.clone(), clock.clone(), ev_tx.clone());
    tokio::spawn(async move {
        loop {
            match net::recv_msg::<ServerMsg>(&mut recv).await {
                Ok(Some(ServerMsg::Pong { client_us, server_us })) => {
                    let mut c = ck.lock().unwrap();
                    c.add(client_us, server_us, now_us());
                    st.lock().unwrap().rtt_us = c.rtt_us();
                }
                Ok(Some(ServerMsg::Rate { video_kbps, fec_pct })) => {
                    let mut s = st.lock().unwrap();
                    s.video_kbps = video_kbps;
                    s.fec_pct = fec_pct;
                }
                Ok(Some(ServerMsg::Rumble { pad, low, high })) => {
                    let _ = tx.send(ClientEvent::Rumble { pad, low, high }).await;
                }
                Ok(Some(ServerMsg::End { reason })) => {
                    let _ = tx.send(ClientEvent::Ended(reason)).await;
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => {
                    let _ = tx.send(ClientEvent::Ended("control stream closed".into())).await;
                    break;
                }
            }
        }
    });

    // Pings.
    let t = ctl_tx.clone();
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(PING_INTERVAL);
        loop {
            iv.tick().await;
            if t.send(ClientMsg::Ping { client_us: now_us() }).is_err() {
                break;
            }
        }
    });

    // Datagram receiver: reassembly, loss handling, reports.
    tokio::spawn(receive_loop(conn.clone(), ev_tx, ctl_tx.clone(), stats.clone(), clock));

    let input = InputSender::start(conn.clone()).await?;
    Ok(Session { start, events: ev_rx, input, stats, conn, ctl_tx, _endpoint: endpoint })
}

async fn receive_loop(
    conn: Connection,
    ev: mpsc::Sender<ClientEvent>,
    ctl: mpsc::UnboundedSender<ClientMsg>,
    stats: Arc<Mutex<Stats>>,
    clock: Arc<Mutex<ClockSync>>,
) {
    let mut video = Assembler::new(Kind::Video, AssemblerConfig::video());
    let mut audio = Assembler::new(Kind::Audio, AssemblerConfig::audio());
    let mut gate = RecoveryGate::default();
    let mut queue = QueueDelay::new(5_000_000);
    let mut report = tokio::time::interval(REPORT_INTERVAL);
    let mut tick = tokio::time::interval(Duration::from_millis(5));
    let mut last_report_us = now_us();
    loop {
        let mut events: Vec<Event> = Vec::new();
        let mut audio_out: Vec<CompleteFrame> = Vec::new();
        tokio::select! {
            d = conn.read_datagram() => {
                let Ok(d) = d else { break };
                let now = now_us();
                match ShardHeader::parse(&d) {
                    Ok((h, payload)) if h.kind == Kind::Video => events = video.push(&h, payload, now),
                    Ok((h, payload)) => {
                        for e in audio.push(&h, payload, now) {
                            if let Event::Frame(f) = e { audio_out.push(f) }
                        }
                    }
                    Err(e) => debug!("bad datagram: {e}"),
                }
            }
            _ = tick.tick() => {
                let now = now_us();
                events = video.poll(now);
                for e in audio.poll(now) {
                    if let Event::Frame(f) = e { audio_out.push(f) }
                }
            }
            _ = report.tick() => {
                let now = now_us();
                let i = video.take_interval();
                let q = queue.take_interval();
                let rtt = clock.lock().unwrap().rtt_us().unwrap_or(0);
                let r = NetworkReport {
                    interval_ms: ((now - last_report_us) / 1000).min(u16::MAX as u64) as u16,
                    shards_received: i.shards_received as u32,
                    shards_missing: i.shards_missing as u32,
                    frames_ok: i.frames_ok as u32,
                    frames_recovered: i.frames_recovered as u32,
                    frames_lost: i.frames_lost as u32,
                    queue_delay_us: q,
                    rtt_us: rtt.min(u32::MAX as u64) as u32,
                };
                last_report_us = now;
                {
                    let mut s = stats.lock().unwrap();
                    s.shards_received += i.shards_received;
                    s.shards_missing += i.shards_missing;
                    s.frames_recovered_by_fec += i.frames_recovered;
                    s.queue_delay_us = q;
                }
                if ctl.send(ClientMsg::Report(r)).is_err() {
                    break;
                }
            }
        }
        for f in audio_out {
            stats.lock().unwrap().audio_packets += 1;
            if ev.try_send(ClientEvent::Audio(f)).is_err() {
                debug!("audio consumer is behind; packet dropped");
            }
        }
        for e in events {
            match e {
                Event::Lost { first, last, speculative } => {
                    debug!(first, last, speculative, "frames lost");
                    gate.on_lost(first, last);
                    let mut s = stats.lock().unwrap();
                    s.frames_lost += (last - first + 1) as u64;
                    s.loss_events += 1;
                    drop(s);
                    let _ = ctl.send(ClientMsg::FrameLost { first, last });
                }
                Event::Frame(f) => {
                    queue.add(f.first_shard_us, f.capture_us);
                    if !gate.admit(&f) {
                        stats.lock().unwrap().frames_dropped_waiting_for_recovery += 1;
                        continue;
                    }
                    {
                        let mut s = stats.lock().unwrap();
                        s.frames_shown += 1;
                        s.bytes_video += f.data.len() as u64;
                        s.last_latency_us = clock.lock().unwrap().to_client(f.capture_us).map(|c| f.complete_us.saturating_sub(c));
                    }
                    // Video must not queue up: if the decoder is behind, wait
                    // for it (bounded channel) rather than build latency silently.
                    if ev.send(ClientEvent::Video(f)).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
    let _ = ev.send(ClientEvent::Ended("connection closed".into())).await;
}

/// Sends input. Keys and buttons go on a reliable ordered stream; mouse motion
/// is added up and sent as one datagram per millisecond (Moonlight does the
/// same: it stops a queue of tiny packets from building up).
#[derive(Clone)]
pub struct InputSender {
    events: mpsc::UnboundedSender<InputEvent>,
    motion: Arc<Mutex<(i32, i32)>>,
    conn: Connection,
    seq: Arc<std::sync::atomic::AtomicU32>,
}

impl InputSender {
    async fn start(conn: Connection) -> anyhow::Result<InputSender> {
        let mut uni = conn.open_uni().await?;
        let (tx, mut rx) = mpsc::unbounded_channel::<InputEvent>();
        tokio::spawn(async move {
            while let Some(e) = rx.recv().await {
                if let Err(err) = net::send_msg(&mut uni, &e).await {
                    warn!("input stream: {err}");
                    break;
                }
            }
        });
        let s = InputSender { events: tx, motion: Arc::new(Mutex::new((0, 0))), conn, seq: Arc::new(0.into()) };
        let flusher = s.clone();
        tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(1));
            iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                iv.tick().await;
                if flusher.flush_motion().is_err() {
                    break;
                }
            }
        });
        Ok(s)
    }

    fn next_seq(&self) -> u32 {
        self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed).wrapping_add(1)
    }

    fn flush_motion(&self) -> anyhow::Result<()> {
        let (dx, dy) = std::mem::take(&mut *self.motion.lock().unwrap());
        if dx == 0 && dy == 0 {
            return Ok(());
        }
        let m = InputDatagram::MouseMove { seq: self.next_seq(), dx: dx.clamp(i16::MIN as i32, i16::MAX as i32) as i16, dy: dy.clamp(i16::MIN as i32, i16::MAX as i32) as i16 };
        self.conn.send_datagram(Bytes::from(m.to_datagram())).map_err(|e| anyhow!("{e}"))
    }

    pub fn mouse_move(&self, dx: i32, dy: i32) {
        let mut m = self.motion.lock().unwrap();
        m.0 = m.0.saturating_add(dx);
        m.1 = m.1.saturating_add(dy);
    }

    pub fn mouse_abs(&self, x: u16, y: u16) {
        let m = InputDatagram::MouseAbs { seq: self.next_seq(), x, y };
        let _ = self.conn.send_datagram(Bytes::from(m.to_datagram()));
    }

    pub fn gamepad_axes(&self, pad: u8, axes: [i16; 6], hat_x: i8, hat_y: i8) {
        let m = InputDatagram::GamepadAxes { seq: self.next_seq(), pad, axes, hat_x, hat_y };
        let _ = self.conn.send_datagram(Bytes::from(m.to_datagram()));
    }

    pub fn send(&self, e: InputEvent) {
        // Motion first, so a click lands where the pointer is.
        let _ = self.flush_motion();
        let _ = self.events.send(e);
    }

    pub fn key(&self, code: u16, down: bool) {
        self.send(InputEvent::Key { code, down });
    }

    pub fn button(&self, button: MouseButton, down: bool) {
        self.send(InputEvent::MouseButton { button, down });
    }
}
