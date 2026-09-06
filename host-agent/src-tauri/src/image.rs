//! Guest base-image digest + Ed25519 signature checks.
//!
//! The platform signs the raw 32-byte SHA-256 of the image with an Ed25519
//! key. The agent pins the **public** key (`IMAGE_SIGNING_PUBLIC_KEY`) and
//! compares the advertised digest to the standby image it already has
//! (`IMAGE_LOCAL_DIGEST`, or a SHA-256 of provided bytes). Mismatch refuses
//! the stub boot path. This module never downloads an image, never holds a
//! private key, and never invokes a hypervisor.

use std::fmt;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use thiserror::Error;

const PUBLIC_KEY_ENV: &str = "IMAGE_SIGNING_PUBLIC_KEY";
const LOCAL_DIGEST_ENV: &str = "IMAGE_LOCAL_DIGEST";

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }

    pub fn parse_hex(value: &str) -> Result<Self, ImageError> {
        let stripped = strip_sha256_prefix(value.trim());
        let decoded = hex::decode(stripped).map_err(|_| ImageError::InvalidDigest)?;
        let bytes: [u8; 32] = decoded.try_into().map_err(|_| ImageError::InvalidDigest)?;
        Ok(Self(bytes))
    }

    pub fn of(image_bytes: &[u8]) -> Self {
        let digest = Sha256::digest(image_bytes);
        Self(digest.into())
    }
}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Sha256Digest").field(&self.to_hex()).finish()
    }
}

/// Advertised by the Matchmaker for a resource class. Digest is 64 hex chars;
/// signature is standard base64 of the 64-byte Ed25519 signature over the
/// raw 32-byte digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageManifest {
    pub digest: Sha256Digest,
    pub signature: Signature,
}

impl ImageManifest {
    pub fn parse(digest_hex: &str, signature_b64: &str) -> Result<Self, ImageError> {
        let digest = Sha256Digest::parse_hex(digest_hex)?;
        let signature = parse_signature(signature_b64)?;
        Ok(Self { digest, signature })
    }

    pub fn digest_hex(&self) -> String {
        self.digest.to_hex()
    }

    pub fn signature_base64(&self) -> String {
        BASE64.encode(self.signature.to_bytes())
    }
}

/// Pinned platform public key plus the digest of the image this host has on
/// standby. Built from env when the provision listener is enabled.
#[derive(Clone, PartialEq, Eq)]
pub struct ImageTrust {
    public_key: VerifyingKey,
    local_digest: Sha256Digest,
}

impl ImageTrust {
    pub fn new(public_key: VerifyingKey, local_digest: Sha256Digest) -> Self {
        Self {
            public_key,
            local_digest,
        }
    }

    pub fn local_digest(&self) -> Sha256Digest {
        self.local_digest
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key.as_bytes())
    }
}

impl fmt::Debug for ImageTrust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageTrust")
            .field("public_key", &self.public_key_hex())
            .field("local_digest", &self.local_digest.to_hex())
            .finish()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ImageError {
    #[error("{PUBLIC_KEY_ENV} is required")]
    MissingPublicKey,
    #[error("{PUBLIC_KEY_ENV} must be a 32-byte Ed25519 public key (64 hex chars)")]
    InvalidPublicKey,
    #[error("{LOCAL_DIGEST_ENV} is required")]
    MissingLocalDigest,
    #[error("{LOCAL_DIGEST_ENV} must be a 64-character SHA-256 hex digest")]
    InvalidLocalDigest,
    #[error("image digest must be a 64-character SHA-256 hex digest")]
    InvalidDigest,
    #[error("image signature must be standard base64 of a 64-byte Ed25519 signature")]
    InvalidSignature,
    #[error("image digest does not match the signed standby image")]
    DigestMismatch,
    #[error("image signature does not verify with the platform public key")]
    SignatureMismatch,
}

/// Load the pinned public key and local standby digest. Fail fast: no in-code
/// fallback and no hardcoded key.
pub fn trust_from_env<F>(mut lookup: F) -> Result<ImageTrust, ImageError>
where
    F: FnMut(&str) -> Option<String>,
{
    let public_raw = nonempty(lookup(PUBLIC_KEY_ENV)).ok_or(ImageError::MissingPublicKey)?;
    let public_key = parse_public_key(&public_raw)?;
    let digest_raw = nonempty(lookup(LOCAL_DIGEST_ENV)).ok_or(ImageError::MissingLocalDigest)?;
    let local_digest =
        Sha256Digest::parse_hex(&digest_raw).map_err(|_| ImageError::InvalidLocalDigest)?;
    Ok(ImageTrust::new(public_key, local_digest))
}

/// Verify advertised digest+signature, then that local standby (or provided
/// bytes) hashes to that digest. Call this **before** [`crate::provision::plan`].
pub fn verify_image(
    trust: &ImageTrust,
    advertised: &ImageManifest,
    image_bytes: Option<&[u8]>,
) -> Result<(), ImageError> {
    if let Some(bytes) = image_bytes {
        let computed = Sha256Digest::of(bytes);
        if !ct_eq(computed.as_bytes(), advertised.digest.as_bytes()) {
            return Err(ImageError::DigestMismatch);
        }
    }
    if !ct_eq(trust.local_digest.as_bytes(), advertised.digest.as_bytes()) {
        return Err(ImageError::DigestMismatch);
    }
    trust
        .public_key
        .verify(advertised.digest.as_bytes(), &advertised.signature)
        .map_err(|_| ImageError::SignatureMismatch)
}

fn parse_public_key(value: &str) -> Result<VerifyingKey, ImageError> {
    let stripped = value.trim();
    let bytes = if let Ok(decoded) = hex::decode(stripped) {
        decoded
    } else {
        BASE64
            .decode(stripped.as_bytes())
            .map_err(|_| ImageError::InvalidPublicKey)?
    };
    let key_bytes: [u8; 32] = bytes.try_into().map_err(|_| ImageError::InvalidPublicKey)?;
    VerifyingKey::from_bytes(&key_bytes).map_err(|_| ImageError::InvalidPublicKey)
}

fn parse_signature(value: &str) -> Result<Signature, ImageError> {
    let stripped = value.trim();
    let decoded = BASE64
        .decode(stripped.as_bytes())
        .map_err(|_| ImageError::InvalidSignature)?;
    let bytes: [u8; 64] = decoded
        .try_into()
        .map_err(|_| ImageError::InvalidSignature)?;
    Ok(Signature::from_bytes(&bytes))
}

fn strip_sha256_prefix(value: &str) -> &str {
    value
        .strip_prefix("sha256:")
        .or_else(|| value.strip_prefix("SHA256:"))
        .unwrap_or(value)
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

fn ct_eq(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right.iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    /// SHA-256 of these bytes is the Standard Lab `.env.example` digest.
    /// Not a real guest disk — CI must not fetch multi-GB images.
    pub(crate) const FIXTURE_STANDARD_BYTES: &[u8] = b"dcl-fixture-standard-v1\n";
    /// Pro Lab fixture bytes. Same software-contract stand-in.
    pub(crate) const FIXTURE_PRO_BYTES: &[u8] = b"dcl-fixture-pro-v1\n";

    /// 32-byte test seed. Not a production platform key. Do not ship as a secret.
    pub(crate) const TEST_SIGNING_SEED: [u8; 32] = *b"test-ed25519-image-key-seed-v001";

    pub(crate) fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&TEST_SIGNING_SEED)
    }

    pub(crate) fn signed_fixture(bytes: &[u8]) -> (ImageTrust, ImageManifest) {
        let signing_key = test_signing_key();
        let digest = Sha256Digest::of(bytes);
        let signature = signing_key.sign(digest.as_bytes());
        let trust = ImageTrust::new(signing_key.verifying_key(), digest);
        (trust, ImageManifest { digest, signature })
    }

    #[test]
    fn valid_signature_accepts_matching_bytes() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        assert_eq!(
            verify_image(&trust, &manifest, Some(FIXTURE_STANDARD_BYTES)),
            Ok(())
        );
        assert_eq!(verify_image(&trust, &manifest, None), Ok(()));
    }

    #[test]
    fn tampered_bytes_are_rejected() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let mut tampered = FIXTURE_STANDARD_BYTES.to_vec();
        tampered[0] ^= 0x01;
        assert_eq!(
            verify_image(&trust, &manifest, Some(&tampered)),
            Err(ImageError::DigestMismatch)
        );
    }

    #[test]
    fn advertised_digest_must_match_local_standby() {
        let (trust, mut manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        manifest.digest = Sha256Digest::of(FIXTURE_PRO_BYTES);
        assert_eq!(
            verify_image(&trust, &manifest, None),
            Err(ImageError::DigestMismatch)
        );
    }

    #[test]
    fn signature_over_tampered_digest_is_rejected() {
        let (trust, mut manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let other = signed_fixture(FIXTURE_PRO_BYTES);
        // Same local digest, but a signature that belongs to a different digest.
        manifest.signature = other.1.signature;
        assert_eq!(
            verify_image(&trust, &manifest, Some(FIXTURE_STANDARD_BYTES)),
            Err(ImageError::SignatureMismatch)
        );
    }

    #[test]
    fn missing_public_key_fails_fast() {
        let error = trust_from_env(|key| match key {
            LOCAL_DIGEST_ENV => Some(Sha256Digest::of(FIXTURE_STANDARD_BYTES).to_hex()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(error, ImageError::MissingPublicKey);
        assert!(error.to_string().contains(PUBLIC_KEY_ENV));
    }

    #[test]
    fn missing_local_digest_fails_fast() {
        let (trust, _) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let error = trust_from_env(|key| match key {
            PUBLIC_KEY_ENV => Some(trust.public_key_hex()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(error, ImageError::MissingLocalDigest);
        assert!(error.to_string().contains(LOCAL_DIGEST_ENV));
    }

    #[test]
    fn invalid_public_key_fails_fast() {
        let error = trust_from_env(|key| match key {
            PUBLIC_KEY_ENV => Some("not-a-key".into()),
            LOCAL_DIGEST_ENV => Some(Sha256Digest::of(FIXTURE_STANDARD_BYTES).to_hex()),
            _ => None,
        })
        .unwrap_err();
        assert_eq!(error, ImageError::InvalidPublicKey);
    }

    #[test]
    fn trust_from_env_accepts_hex_public_key() {
        let (trust, manifest) = signed_fixture(FIXTURE_STANDARD_BYTES);
        let loaded = trust_from_env(|key| match key {
            PUBLIC_KEY_ENV => Some(trust.public_key_hex()),
            LOCAL_DIGEST_ENV => Some(trust.local_digest().to_hex()),
            _ => None,
        })
        .unwrap();
        assert_eq!(
            verify_image(&loaded, &manifest, Some(FIXTURE_STANDARD_BYTES)),
            Ok(())
        );
    }

    #[test]
    fn sha256_prefix_is_accepted() {
        let digest = Sha256Digest::of(FIXTURE_STANDARD_BYTES);
        let parsed = Sha256Digest::parse_hex(&format!("sha256:{}", digest.to_hex())).unwrap();
        assert_eq!(parsed, digest);
    }

    #[test]
    fn fixture_digests_are_stable() {
        assert_eq!(
            Sha256Digest::of(FIXTURE_STANDARD_BYTES).to_hex(),
            "6ed707af13eb04a928001996ae5fc7a49ab405e73534caacf86ad65c51bab8fe"
        );
        assert_eq!(
            Sha256Digest::of(FIXTURE_PRO_BYTES).to_hex(),
            "b5dc04d095774efc8fee56dfcbabbe7346788f29a7576aaa27248d9b8249b118"
        );
    }

    /// Local-dev / pytest fixtures. Seed is test-only; never a production key.
    #[test]
    fn local_dev_image_vectors_are_stable() {
        let signing_key = test_signing_key();
        let public_hex = hex::encode(signing_key.verifying_key().as_bytes());
        let standard = Sha256Digest::of(FIXTURE_STANDARD_BYTES);
        let pro = Sha256Digest::of(FIXTURE_PRO_BYTES);
        let standard_sig = BASE64.encode(signing_key.sign(standard.as_bytes()).to_bytes());
        let pro_sig = BASE64.encode(signing_key.sign(pro.as_bytes()).to_bytes());
        eprintln!("IMAGE_SIGNING_PUBLIC_KEY={public_hex}");
        eprintln!("IMAGE_STANDARD_DIGEST={}", standard.to_hex());
        eprintln!("IMAGE_STANDARD_SIGNATURE={standard_sig}");
        eprintln!("IMAGE_PRO_DIGEST={}", pro.to_hex());
        eprintln!("IMAGE_PRO_SIGNATURE={pro_sig}");
        assert_eq!(public_hex.len(), 64);
        assert_eq!(standard.to_hex().len(), 64);
        assert_eq!(pro.to_hex().len(), 64);
        assert_eq!(standard_sig.len(), 88);
        assert_eq!(pro_sig.len(), 88);
        assert_eq!(
            standard.to_hex(),
            "6ed707af13eb04a928001996ae5fc7a49ab405e73534caacf86ad65c51bab8fe"
        );
        assert_eq!(
            pro.to_hex(),
            "b5dc04d095774efc8fee56dfcbabbe7346788f29a7576aaa27248d9b8249b118"
        );
        assert_eq!(
            public_hex,
            "7dadc44f5a3f883a46746bfdade30924db20fe6b43c8469733cfb007279d0da3"
        );
        assert_eq!(
            standard_sig,
            "WQjS2zyW+BjgfoZDK3IFx6OIwXPmRxO22PN4FpfEJNpjY/AgEsLmqLVak99QZyY14YKNI5hiARiPkulHNpsrCQ=="
        );
        assert_eq!(
            pro_sig,
            "T1FrV8mlhTvhk+0laH74qnb6s6CTM2M+waEhIAtjszKlN3pZyQ+w18xDfp0xvbA3MO+FXvt8Z+B+hmXDrvBaDA=="
        );
    }
}
