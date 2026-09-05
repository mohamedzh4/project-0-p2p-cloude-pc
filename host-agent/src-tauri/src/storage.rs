//! Guest rclone mount plan for renterd S3 (phase 15).
//!
//! Persistent files are decrypted/mounted **inside the guest**, never as
//! plaintext on the volunteer host disk. This module only builds rclone
//! arguments from public locator env (`S3_ENDPOINT`, `S3_BUCKET`, prefix).
//! It does not run rclone and it must not read guest S3 credentials.

use std::fmt;

use thiserror::Error;

const DEFAULT_MOUNT_POINT: &str = "/home/user/Persistent";
const DEFAULT_REGION: &str = "us-east-1";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StorageError {
    #[error("{0} is not set. The Host Agent does not hardcode a renterd S3 endpoint or bucket.")]
    MissingEnv(&'static str),
    #[error("S3_ENDPOINT must be an http or https URL")]
    InvalidEndpoint,
}

/// rclone mount plan for injection into the guest. Secrets stay in guest env.
#[derive(PartialEq, Eq)]
pub struct GuestRcloneMount {
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    pub region: String,
    pub mount_point: String,
}

impl fmt::Debug for GuestRcloneMount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GuestRcloneMount")
            .field("endpoint", &self.endpoint)
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .field("region", &self.region)
            .field("mount_point", &self.mount_point)
            .finish()
    }
}

impl GuestRcloneMount {
    /// Opt-in: unset `S3_ENDPOINT` skips the plan (UI-only machines).
    pub fn from_env<F>(mut lookup: F) -> Result<Option<Self>, StorageError>
    where
        F: FnMut(&str) -> Option<String>,
    {
        let Some(endpoint) = nonempty(lookup("S3_ENDPOINT")) else {
            return Ok(None);
        };
        if !(endpoint.starts_with("http://") || endpoint.starts_with("https://")) {
            return Err(StorageError::InvalidEndpoint);
        }
        let bucket = nonempty(lookup("S3_BUCKET")).ok_or(StorageError::MissingEnv("S3_BUCKET"))?;
        let prefix = nonempty(lookup("S3_PREFIX")).unwrap_or_default();
        let region = nonempty(lookup("S3_REGION")).unwrap_or_else(|| DEFAULT_REGION.to_string());
        let mount_point =
            nonempty(lookup("S3_GUEST_MOUNT")).unwrap_or_else(|| DEFAULT_MOUNT_POINT.to_string());
        Ok(Some(Self {
            endpoint: endpoint.trim_end_matches('/').to_string(),
            bucket,
            prefix,
            region,
            mount_point,
        }))
    }

    pub fn rclone_args(&self) -> Vec<String> {
        let remote = if self.prefix.is_empty() {
            format!(":s3:{}", self.bucket)
        } else {
            format!(":s3:{}/{}", self.bucket, self.prefix.trim_matches('/'))
        };
        vec![
            "mount".to_string(),
            remote,
            self.mount_point.clone(),
            "--s3-provider".to_string(),
            "Other".to_string(),
            "--s3-endpoint".to_string(),
            self.endpoint.clone(),
            "--s3-env-auth".to_string(),
            "--s3-force-path-style".to_string(),
            "--s3-region".to_string(),
            self.region.clone(),
            "--s3-chunk-size".to_string(),
            "120MiB".to_string(),
            "--fast-list".to_string(),
            "--vfs-cache-mode".to_string(),
            "full".to_string(),
        ]
    }
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lookup(key: &str) -> Option<String> {
        match key {
            "S3_ENDPOINT" => Some("http://renterd.test.invalid:8081".to_string()),
            "S3_BUCKET" => Some("lab-files".to_string()),
            "S3_PREFIX" => Some("users/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/".to_string()),
            "S3_REGION" => Some("us-east-1".to_string()),
            _ => None,
        }
    }

    #[test]
    fn unset_endpoint_skips_the_plan() {
        assert!(GuestRcloneMount::from_env(|_| None).unwrap().is_none());
    }

    #[test]
    fn missing_bucket_fails_fast_when_endpoint_is_set() {
        let error = GuestRcloneMount::from_env(|key| {
            (key == "S3_ENDPOINT").then(|| "http://renterd.test.invalid".to_string())
        })
        .unwrap_err();
        assert!(matches!(error, StorageError::MissingEnv("S3_BUCKET")));
    }

    #[test]
    fn plan_builds_rclone_args_without_reading_secret_key() {
        let plan = GuestRcloneMount::from_env(lookup).unwrap().unwrap();
        let args = plan.rclone_args();
        assert_eq!(args[0], "mount");
        assert_eq!(
            args[1],
            ":s3:lab-files/users/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
        );
        assert!(args.contains(&"--vfs-cache-mode".to_string()));
        assert!(args.contains(&"full".to_string()));
        assert!(args.contains(&"--s3-env-auth".to_string()));
        assert!(args.contains(&"--s3-force-path-style".to_string()));
        assert!(args.contains(&"http://renterd.test.invalid:8081".to_string()));
        let debug = format!("{plan:?}");
        assert!(!debug.to_lowercase().contains("secret"));
        assert!(!args.iter().any(|a| a.contains("SECRET") || a.contains("secret")));
    }

    #[test]
    fn production_source_does_not_read_s3_secret_key() {
        let source = include_str!("storage.rs");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("storage.rs should have a test module");
        assert!(
            !production.contains("S3_SECRET_KEY"),
            "Host Agent must not read the renterd S3 secret; keys belong in the guest"
        );
        assert!(!production.contains("S3_ACCESS_KEY"));
        assert!(!production.contains("Command::new(\"rclone\")"));
    }
}
