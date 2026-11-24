//! Session token proof and certificate fingerprints.
//!
//! The Matchmaker makes a 32-byte token per booking. The guest gets it in its
//! bootstrap, the renter's client gets it over HTTPS. The token itself never
//! crosses the stream connection: the client proves it knows it with
//! HMAC-SHA256(token, label ‖ server nonce ‖ TLS exporter). The exporter is
//! unique to this TLS session, so a proof cannot be replayed on another
//! connection or relayed by a man in the middle.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const TOKEN_LEN: usize = 32;
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-dcl-stream-auth";
const PROOF_LABEL: &[u8] = b"dcl-stream/1 auth";

pub type Token = [u8; TOKEN_LEN];

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("token must be {TOKEN_LEN} bytes as 64 hex characters")]
    BadToken,
    #[error("fingerprint must be 64 hex characters")]
    BadFingerprint,
}

fn mac(token: &Token) -> Hmac<Sha256> {
    Hmac::<Sha256>::new_from_slice(token).expect("HMAC takes any key length")
}

pub fn proof(token: &Token, nonce: &[u8; 32], exporter: &[u8; 32]) -> [u8; 32] {
    let mut m = mac(token);
    m.update(PROOF_LABEL);
    m.update(nonce);
    m.update(exporter);
    m.finalize().into_bytes().into()
}

/// Constant-time check.
pub fn verify(token: &Token, nonce: &[u8; 32], exporter: &[u8; 32], proof: &[u8; 32]) -> bool {
    let mut m = mac(token);
    m.update(PROOF_LABEL);
    m.update(nonce);
    m.update(exporter);
    m.verify_slice(proof).is_ok()
}

pub fn parse_token(hex_str: &str) -> Result<Token, AuthError> {
    let v = hex::decode(hex_str.trim()).map_err(|_| AuthError::BadToken)?;
    v.try_into().map_err(|_| AuthError::BadToken)
}

pub fn new_token() -> Token {
    use rand::RngCore;
    let mut t = [0u8; TOKEN_LEN];
    rand::rngs::OsRng.fill_bytes(&mut t);
    t
}

/// Seed of the server's Ed25519 key, derived from the session token.
///
/// Both ends can compute the server certificate from the token alone, so the
/// guest never has to report a fingerprint back: the client pins the
/// certificate it expects. Whoever knows the token could act as the server,
/// but that is only the renter (no gain), the Matchmaker (trusted) and the
/// guest itself; the host can read guest memory anyway (threat model).
pub fn server_key_seed(token: &Token) -> [u8; 32] {
    let mut m = Hmac::<Sha256>::new_from_slice(b"dcl-stream/1 server key").expect("any key length");
    m.update(token);
    m.finalize().into_bytes().into()
}

/// SHA-256 of a DER certificate: what the client pins.
pub fn fingerprint(cert_der: &[u8]) -> [u8; 32] {
    Sha256::digest(cert_der).into()
}

pub fn parse_fingerprint(hex_str: &str) -> Result<[u8; 32], AuthError> {
    let cleaned: String = hex_str.trim().chars().filter(|c| *c != ':').collect();
    let v = hex::decode(cleaned).map_err(|_| AuthError::BadFingerprint)?;
    v.try_into().map_err(|_| AuthError::BadFingerprint)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proof_checks_token_nonce_and_exporter() {
        let t = [1u8; 32];
        let p = proof(&t, &[2; 32], &[3; 32]);
        assert!(verify(&t, &[2; 32], &[3; 32], &p));
        assert!(!verify(&[9; 32], &[2; 32], &[3; 32], &p), "other token");
        assert!(!verify(&t, &[4; 32], &[3; 32], &p), "other nonce (replay)");
        assert!(!verify(&t, &[2; 32], &[5; 32], &p), "other TLS session (relay)");
    }

    #[test]
    fn parsing() {
        let t = new_token();
        assert_eq!(parse_token(&hex::encode(t)).unwrap(), t);
        assert_ne!(new_token(), t);
        assert_eq!(parse_token("abcd"), Err(AuthError::BadToken));
        assert_eq!(parse_token(&"zz".repeat(32)), Err(AuthError::BadToken));
        let f = fingerprint(b"cert");
        let colons = hex::encode(f).as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(":");
        assert_eq!(parse_fingerprint(&colons).unwrap(), f);
        assert_eq!(parse_fingerprint("00"), Err(AuthError::BadFingerprint));
    }
}
