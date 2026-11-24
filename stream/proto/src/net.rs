//! QUIC setup shared by server and client: TLS with a pinned self-signed
//! certificate, transport tuning for real-time video, message framing on
//! streams, and the token handshake.

use crate::auth::{self, Token, EXPORTER_LABEL};
use crate::msg::{self, Capabilities, ClientMsg, ServerMsg, MAX_MSG_BYTES};
use crate::wire::{ALPN, PROTOCOL_VERSION};
use quinn::{Connection, RecvStream, SendStream};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

pub const SERVER_NAME: &str = "dcl-stream";
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum NetError {
    #[error("TLS setup: {0}")]
    Tls(String),
    #[error("connection: {0}")]
    Connection(#[from] quinn::ConnectionError),
    #[error("stream write: {0}")]
    Write(#[from] quinn::WriteError),
    #[error("stream read: {0}")]
    Read(#[from] quinn::ReadExactError),
    #[error("message: {0}")]
    Msg(#[from] msg::MsgError),
    #[error("peer sent a message over {MAX_MSG_BYTES} bytes")]
    TooLarge,
    #[error("authentication failed")]
    AuthFailed,
    #[error("protocol: {0}")]
    Protocol(&'static str),
    #[error("timed out")]
    Timeout,
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// The session's server certificate, the same every time for one token:
/// Ed25519 (deterministic signatures) with fixed fields. See
/// `auth::server_key_seed` for why this is safe here.
pub fn cert_from_token(token: &Token) -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), NetError> {
    let tls = |e: rcgen::Error| NetError::Tls(e.to_string());
    // PKCS#8 v1 wrapper for a raw Ed25519 seed (RFC 8410).
    let mut pkcs8 = vec![0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20];
    pkcs8.extend_from_slice(&auth::server_key_seed(token));
    let der = PrivatePkcs8KeyDer::from(pkcs8);
    let kp = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(&der, &rcgen::PKCS_ED25519).map_err(tls)?;
    let mut params = rcgen::CertificateParams::new(vec![SERVER_NAME.to_string()]).map_err(tls)?;
    params.not_before = rcgen::date_time_ymd(2025, 1, 1);
    params.not_after = rcgen::date_time_ymd(2099, 12, 31);
    params.serial_number = Some(rcgen::SerialNumber::from(vec![1u8]));
    let mut dn = rcgen::DistinguishedName::new();
    dn.push(rcgen::DnType::CommonName, SERVER_NAME);
    params.distinguished_name = dn;
    let cert = params.self_signed(&kp).map_err(tls)?;
    Ok((cert.der().clone(), PrivateKeyDer::Pkcs8(der)))
}

/// The fingerprint a client should pin for this token.
pub fn fingerprint_for_token(token: &Token) -> Result<[u8; 32], NetError> {
    Ok(auth::fingerprint(cert_from_token(token)?.0.as_ref()))
}

/// A fresh random self-signed certificate (not tied to a token).
pub fn generate_cert() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>), NetError> {
    let ck = rcgen::generate_simple_self_signed(vec![SERVER_NAME.to_string()]).map_err(|e| NetError::Tls(e.to_string()))?;
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(ck.key_pair.serialize_der()));
    Ok((ck.cert.der().clone(), key))
}

/// Transport tuned for a video stream:
/// - BBR, not loss-based Cubic/NewReno: random loss must not halve the window
///   (FEC handles it); BBR follows measured bandwidth and RTT.
/// - big datagram buffers so a keyframe burst is not dropped locally.
/// - fixed 1200-byte packets, no path-MTU discovery: the mesh MTU is 1280, so
///   there is nothing to gain, and a lost probe would shrink the packet size
///   under shards already cut for the bigger size.
pub fn transport() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    t.max_idle_timeout(Some(Duration::from_secs(10).try_into().expect("10 s fits")));
    t.keep_alive_interval(Some(Duration::from_secs(1)));
    t.datagram_receive_buffer_size(Some(8 * 1024 * 1024));
    t.datagram_send_buffer_size(4 * 1024 * 1024);
    t.max_concurrent_bidi_streams(2u32.into());
    t.max_concurrent_uni_streams(4u32.into());
    t.initial_mtu(1200);
    t.min_mtu(1200);
    t.mtu_discovery_config(None);
    t.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    t
}

pub fn server_config(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> Result<quinn::ServerConfig, NetError> {
    let mut tls = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NetError::Tls(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| NetError::Tls(e.to_string()))?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.max_early_data_size = 0;
    let quic = quinn::crypto::rustls::QuicServerConfig::try_from(tls).map_err(|e| NetError::Tls(e.to_string()))?;
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(quic));
    cfg.transport_config(Arc::new(transport()));
    Ok(cfg)
}

/// Client config that accepts exactly one server certificate: the one whose
/// SHA-256 the Matchmaker handed out for this booking.
pub fn client_config(pinned: [u8; 32]) -> Result<quinn::ClientConfig, NetError> {
    let p = provider();
    let mut tls = rustls::ClientConfig::builder_with_provider(p.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| NetError::Tls(e.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCert { pinned, provider: p }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls).map_err(|e| NetError::Tls(e.to_string()))?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(quic));
    cfg.transport_config(Arc::new(transport()));
    Ok(cfg)
}

#[derive(Debug)]
struct PinnedCert {
    pinned: [u8; 32],
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedCert {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        use subtle::ConstantTimeEq;
        if bool::from(auth::fingerprint(end_entity.as_ref()).ct_eq(&self.pinned)) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(rustls::CertificateError::ApplicationVerificationFailure))
        }
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::PeerIncompatible(rustls::PeerIncompatible::Tls13RequiredForQuic))
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

pub async fn send_msg<T: Serialize>(s: &mut SendStream, m: &T) -> Result<(), NetError> {
    s.write_all(&msg::frame(m)?).await?;
    Ok(())
}

/// Next message, or None when the peer finished the stream cleanly.
pub async fn recv_msg<T: DeserializeOwned>(r: &mut RecvStream) -> Result<Option<T>, NetError> {
    let mut len = [0u8; 4];
    match r.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = u32::from_le_bytes(len) as usize;
    if n > MAX_MSG_BYTES {
        return Err(NetError::TooLarge);
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body).await?;
    Ok(Some(msg::decode(&body)?))
}

fn exporter(conn: &Connection) -> Result<[u8; 32], NetError> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, EXPORTER_LABEL, b"").map_err(|_| NetError::Tls("exporter".into()))?;
    Ok(out)
}

pub struct ControlStream {
    pub send: SendStream,
    pub recv: RecvStream,
}

/// Server side: the client must open the control stream and prove the token
/// within AUTH_TIMEOUT. Nothing but Hello is sent before that.
pub async fn server_handshake(conn: &Connection, token: &Token, caps: Capabilities) -> Result<ControlStream, NetError> {
    tokio::time::timeout(AUTH_TIMEOUT, async {
        let (mut send, mut recv) = conn.accept_bi().await?;
        match recv_msg::<ClientMsg>(&mut recv).await? {
            Some(ClientMsg::Open { version }) if version == PROTOCOL_VERSION => {}
            Some(ClientMsg::Open { .. }) => return Err(NetError::Protocol("protocol version mismatch")),
            _ => return Err(NetError::Protocol("first message must be Open")),
        }
        let nonce = auth::new_token();
        send_msg(&mut send, &ServerMsg::Hello { version: PROTOCOL_VERSION, nonce, caps }).await?;
        let Some(ClientMsg::Auth { proof }) = recv_msg::<ClientMsg>(&mut recv).await? else {
            return Err(NetError::Protocol("first message must be Auth"));
        };
        if !auth::verify(token, &nonce, &exporter(conn)?, &proof) {
            return Err(NetError::AuthFailed);
        }
        send_msg(&mut send, &ServerMsg::AuthOk).await?;
        Ok(ControlStream { send, recv })
    })
    .await
    .map_err(|_| NetError::Timeout)?
}

pub async fn client_handshake(conn: &Connection, token: &Token) -> Result<(ControlStream, Capabilities), NetError> {
    tokio::time::timeout(AUTH_TIMEOUT, async {
        let (mut send, mut recv) = conn.open_bi().await?;
        // The peer learns about a stream only when data arrives on it, so the client speaks first.
        send_msg(&mut send, &ClientMsg::Open { version: PROTOCOL_VERSION }).await?;
        let Some(ServerMsg::Hello { version, nonce, caps }) = recv_msg::<ServerMsg>(&mut recv).await? else {
            return Err(NetError::Protocol("expected Hello"));
        };
        if version != PROTOCOL_VERSION {
            return Err(NetError::Protocol("protocol version mismatch"));
        }
        let proof = auth::proof(token, &nonce, &exporter(conn)?);
        send_msg(&mut send, &ClientMsg::Auth { proof }).await?;
        match recv_msg::<ServerMsg>(&mut recv).await? {
            Some(ServerMsg::AuthOk) => Ok((ControlStream { send, recv }, caps)),
            _ => Err(NetError::AuthFailed),
        }
    })
    .await
    .map_err(|_| NetError::Timeout)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_certificate_is_stable_and_unique() {
        let t = auth::new_token();
        let (a, _) = cert_from_token(&t).unwrap();
        let (b, _) = cert_from_token(&t).unwrap();
        assert_eq!(a.as_ref(), b.as_ref(), "same token, same certificate");
        assert_eq!(fingerprint_for_token(&t).unwrap(), auth::fingerprint(a.as_ref()));
        assert_ne!(fingerprint_for_token(&auth::new_token()).unwrap(), fingerprint_for_token(&t).unwrap());
        // and rustls accepts it as a server certificate
        let (c, k) = cert_from_token(&t).unwrap();
        server_config(c, k).unwrap();
    }
}
