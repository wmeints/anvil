//! Secrets that sandboxes use without seeing their values.
//!
//! The daemon keeps the secrets in a YAML file only the user can read, and adds them to
//! sandboxes through microsandbox's secrets feature. The guest sees a placeholder such as
//! `$MSB_GH_TOKEN` in the secret's environment variable, and microsandbox's TLS proxy on the
//! host replaces the placeholder with the real value in requests to the secret's allowed hosts.

use microsandbox::MicrosandboxResult;
use microsandbox::sandbox::{SandboxBuilder, SandboxHandle};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use thiserror::Error;

/// Prefix microsandbox reserves for its own environment variables.
const RESERVED_PREFIX: &str = "MSB_";

/// Errors that can occur while validating or storing secrets.
#[derive(Error, Debug)]
pub enum SecretError {
    #[error(
        "invalid secret name {0:?}: use letters, digits and underscores, don't start with a digit or {RESERVED_PREFIX}"
    )]
    InvalidName(String),
    #[error("the value of secret {0} is empty")]
    EmptyValue(String),
    #[error("invalid allowed host {0:?}: use a host name such as api.example.com or *.example.com")]
    InvalidHost(String),
    #[error("secret {0} has no default allowed hosts, so name the hosts that may receive it")]
    NoAllowedHosts(String),
    #[error("failed to read secrets from {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse secrets in {path}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to encode secrets")]
    Encode(#[source] serde_yaml::Error),
    #[error("secret {name} appears more than once in {path}")]
    Duplicate { path: PathBuf, name: String },
    #[error("invalid secret in {path}")]
    Invalid {
        path: PathBuf,
        #[source]
        source: Box<SecretError>,
    },
    #[error("failed to write secrets to {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// A secret: the environment variable that exposes it, its value and the hosts that may
/// receive the value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Secret {
    name: String,
    value: String,
    allowed_hosts: Vec<String>,
}

impl Secret {
    /// Validates a secret. Without allowed hosts, it uses the defaults for well-known names
    /// such as `GH_TOKEN` and `ANTHROPIC_API_KEY`.
    pub fn new(
        name: String,
        value: String,
        allowed_hosts: Vec<String>,
    ) -> Result<Self, SecretError> {
        validate_name(&name)?;

        if value.is_empty() {
            return Err(SecretError::EmptyValue(name));
        }

        let allowed_hosts = if allowed_hosts.is_empty() {
            default_allowed_hosts(&name)
                .iter()
                .map(|host| host.to_string())
                .collect()
        } else {
            allowed_hosts
        };

        if allowed_hosts.is_empty() {
            return Err(SecretError::NoAllowedHosts(name));
        }

        if let Some(host) = allowed_hosts.iter().find(|host| !is_valid_host(host)) {
            return Err(SecretError::InvalidHost(host.clone()));
        }

        Ok(Self {
            name,
            value,
            allowed_hosts,
        })
    }

    /// Returns the environment variable that holds the placeholder in the sandbox.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the hosts that may receive the real value.
    pub fn allowed_hosts(&self) -> &[String] {
        &self.allowed_hosts
    }
}

/// Leaves the value out, so secrets don't end up in logs.
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .field("allowed_hosts", &self.allowed_hosts)
            .finish()
    }
}

/// Checks that a secret name is an environment variable name that microsandbox doesn't reserve.
pub fn validate_name(name: &str) -> Result<(), SecretError> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(RESERVED_PREFIX);

    if valid {
        Ok(())
    } else {
        Err(SecretError::InvalidName(name.to_string()))
    }
}

/// Checks that a host is a host name, optionally with a `*.` prefix for its subdomains.
fn is_valid_host(host: &str) -> bool {
    let name = host.strip_prefix("*.").unwrap_or(host);

    !name.is_empty()
        && name
            .split('.')
            .all(|label| !label.is_empty() && label.chars().all(is_host_char))
}

fn is_host_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

/// Returns the hosts the tools that read a well-known secret send it to.
fn default_allowed_hosts(name: &str) -> &'static [&'static str] {
    match name {
        "GH_TOKEN" | "GITHUB_TOKEN" => &["github.com", "api.github.com", "uploads.github.com"],
        "COPILOT_GITHUB_TOKEN" => &["github.com", "api.github.com", "*.githubcopilot.com"],
        "ANTHROPIC_API_KEY" | "CLAUDE_CODE_OAUTH_TOKEN" => &["api.anthropic.com"],
        _ => &[],
    }
}

/// The YAML file the daemon keeps secrets in. Only the user can read it.
pub struct SecretStore {
    path: PathBuf,
    // Serializes read-modify-write cycles of concurrent requests.
    lock: Mutex<()>,
}

impl SecretStore {
    /// Opens the store at `path`. The file is created on the first `set`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    /// Returns all secrets, or none when the file doesn't exist yet.
    pub fn load(&self) -> Result<Vec<Secret>, SecretError> {
        let content = match fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(vec![]),
            Err(source) => {
                return Err(SecretError::Read {
                    path: self.path.clone(),
                    source,
                });
            }
        };

        let secrets: Vec<Secret> =
            serde_yaml::from_str(&content).map_err(|source| SecretError::Parse {
                path: self.path.clone(),
                source,
            })?;

        // The file can be edited by hand, so check it like new secrets.
        let secrets: Vec<Secret> = secrets
            .into_iter()
            .map(|secret| Secret::new(secret.name, secret.value, secret.allowed_hosts))
            .collect::<Result<_, _>>()
            .map_err(|source| SecretError::Invalid {
                path: self.path.clone(),
                source: Box::new(source),
            })?;

        let mut names = HashSet::new();
        if let Some(secret) = secrets.iter().find(|secret| !names.insert(&secret.name)) {
            return Err(SecretError::Duplicate {
                path: self.path.clone(),
                name: secret.name.clone(),
            });
        }

        Ok(secrets)
    }

    /// Adds a secret, or replaces the secret with the same name.
    pub fn set(&self, secret: Secret) -> Result<(), SecretError> {
        self.update(|secrets| {
            secrets.retain(|existing| existing.name != secret.name);
            secrets.push(secret);
            true
        })?;

        Ok(())
    }

    /// Removes the secret with the given name. Returns whether the secret existed.
    pub fn remove(&self, name: &str) -> Result<bool, SecretError> {
        self.update(|secrets| {
            let count = secrets.len();
            secrets.retain(|existing| existing.name != name);
            secrets.len() < count
        })
    }

    /// Loads the secrets and lets `change` change them. Writes them back and returns `true`
    /// when `change` returns `true`.
    fn update(&self, change: impl FnOnce(&mut Vec<Secret>) -> bool) -> Result<bool, SecretError> {
        // The lock guards no data, so a panic while holding it leaves nothing inconsistent.
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);

        let mut secrets = self.load()?;
        if !change(&mut secrets) {
            return Ok(false);
        }

        let content = serde_yaml::to_string(&secrets).map_err(SecretError::Encode)?;

        write_private(&self.path, &content).map_err(|source| SecretError::Write {
            path: self.path.clone(),
            source,
        })?;

        Ok(true)
    }
}

/// Replaces a file with `content` that only the user can read, without leaving a partial file.
fn write_private(path: &Path, content: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }

    let temp_path = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp_path)?;

    // The mode only applies to new files, so fix a temp file left over with other permissions.
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(content.as_bytes())?;
    file.sync_all()?;

    fs::rename(&temp_path, path)
}

/// Adds the secrets to a sandbox that is being created.
pub fn add_to_builder(builder: SandboxBuilder, secrets: &[Secret]) -> SandboxBuilder {
    secrets.iter().fold(builder, |builder, secret| {
        builder.secret(|s| {
            let s = s.env(&secret.name).value(&secret.value);
            secret.allowed_hosts.iter().fold(s, |s, host| s.allow(host))
        })
    })
}

/// Adds or replaces a secret in an existing sandbox. A running sandbox picks it up the next
/// time it starts.
pub async fn add_to_sandbox(handle: &SandboxHandle, secret: &Secret) -> MicrosandboxResult<()> {
    handle
        .modify()
        .secret(|s| {
            let s = s.env(&secret.name).value(&secret.value);
            secret.allowed_hosts.iter().fold(s, |s, host| s.allow(host))
        })
        .next_start()
        .apply()
        .await?;

    Ok(())
}

/// Removes a secret from an existing sandbox. A running sandbox keeps the secret until it
/// restarts. Removing a secret the sandbox doesn't have does nothing.
pub async fn remove_from_sandbox(handle: &SandboxHandle, name: &str) -> MicrosandboxResult<()> {
    handle
        .modify()
        .remove_secret(name)
        .next_start()
        .apply()
        .await?;

    Ok(())
}

/// Makes the microsandbox database readable by the user only, because microsandbox stores
/// the secret values in it.
pub fn protect_database(msb_home: &Path) -> io::Result<()> {
    let dir = msb_home.join("db");

    fs::create_dir_all(&dir)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(name: &str, hosts: &[&str]) -> Result<Secret, SecretError> {
        Secret::new(
            name.to_string(),
            "value".to_string(),
            hosts.iter().map(|host| host.to_string()).collect(),
        )
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn new_accepts_environment_variable_names() {
        for name in ["GH_TOKEN", "_X", "a1", "My_Var_2"] {
            assert!(secret(name, &["example.com"]).is_ok(), "{name}");
        }
    }

    #[test]
    fn new_rejects_invalid_and_reserved_names() {
        for name in ["", "1ABC", "GH-TOKEN", "A=B", "A B", "MSB_TOKEN", "TÖKEN"] {
            assert!(
                matches!(
                    secret(name, &["example.com"]),
                    Err(SecretError::InvalidName(_))
                ),
                "{name}"
            );
        }
    }

    #[test]
    fn new_rejects_empty_value() {
        let result = Secret::new("X".into(), String::new(), vec!["example.com".into()]);

        assert!(matches!(result, Err(SecretError::EmptyValue(_))));
    }

    #[test]
    fn new_uses_default_hosts_for_well_known_names() {
        assert_eq!(
            secret("GH_TOKEN", &[]).unwrap().allowed_hosts,
            ["github.com", "api.github.com", "uploads.github.com"]
        );
        assert_eq!(
            secret("COPILOT_GITHUB_TOKEN", &[]).unwrap().allowed_hosts,
            ["github.com", "api.github.com", "*.githubcopilot.com"]
        );
        assert_eq!(
            secret("CLAUDE_CODE_OAUTH_TOKEN", &[])
                .unwrap()
                .allowed_hosts,
            ["api.anthropic.com"]
        );
    }

    #[test]
    fn new_prefers_given_hosts_over_defaults() {
        assert_eq!(
            secret("GH_TOKEN", &["ghe.example.com"])
                .unwrap()
                .allowed_hosts,
            ["ghe.example.com"]
        );
    }

    #[test]
    fn new_requires_hosts_for_unknown_names() {
        assert!(matches!(
            secret("MY_TOKEN", &[]),
            Err(SecretError::NoAllowedHosts(_))
        ));
    }

    #[test]
    fn new_rejects_invalid_hosts() {
        for host in [
            "*",
            "",
            "*.",
            "https://example.com",
            "a..b",
            "a b",
            "host:443",
        ] {
            assert!(
                matches!(secret("X", &[host]), Err(SecretError::InvalidHost(_))),
                "{host}"
            );
        }
    }

    #[test]
    fn new_accepts_wildcard_subdomains() {
        assert!(secret("X", &["*.example.com"]).is_ok());
    }

    #[test]
    fn debug_leaves_out_the_value() {
        let secret = Secret::new("X".into(), "hunter2".into(), vec!["example.com".into()]).unwrap();

        assert!(!format!("{secret:?}").contains("hunter2"));
    }

    #[test]
    fn load_returns_nothing_without_file() {
        let dir = tempfile::tempdir().unwrap();

        let store = SecretStore::new(dir.path().join("secrets.yml"));

        assert!(store.load().unwrap().is_empty());
    }

    #[test]
    fn set_stores_secrets_readable_by_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("anvil/secrets.yml");
        let store = SecretStore::new(&path);

        store.set(secret("GH_TOKEN", &[]).unwrap()).unwrap();
        store.set(secret("X", &["example.com"]).unwrap()).unwrap();

        let names: Vec<_> = store.load().unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["GH_TOKEN", "X"]);
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn set_replaces_secret_with_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.yml"));

        store.set(secret("X", &["a.example.com"]).unwrap()).unwrap();
        let updated = Secret::new("X".into(), "new".into(), vec!["b.example.com".into()]).unwrap();
        store.set(updated.clone()).unwrap();

        assert_eq!(store.load().unwrap(), [updated]);
    }

    #[test]
    fn remove_deletes_only_the_named_secret() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");
        let store = SecretStore::new(&path);
        store.set(secret("A", &["example.com"]).unwrap()).unwrap();
        store.set(secret("B", &["example.com"]).unwrap()).unwrap();

        assert!(store.remove("A").unwrap());

        let names: Vec<_> = store.load().unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["B"]);
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn remove_reports_missing_secret() {
        let dir = tempfile::tempdir().unwrap();
        let store = SecretStore::new(dir.path().join("secrets.yml"));
        store.set(secret("A", &["example.com"]).unwrap()).unwrap();

        assert!(!store.remove("B").unwrap());
        assert_eq!(store.load().unwrap().len(), 1);
    }

    #[test]
    fn remove_without_file_creates_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");

        assert!(!SecretStore::new(&path).remove("A").unwrap());
        assert!(!path.exists());
    }

    #[test]
    fn load_rejects_invalid_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");
        fs::write(&path, "- name: X\n  value: v\n  allowed_hosts: ['*']\n").unwrap();

        let result = SecretStore::new(&path).load();

        assert!(matches!(result, Err(SecretError::Invalid { .. })));
    }

    #[test]
    fn load_rejects_duplicate_names() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");
        let entry = "- name: X\n  value: v\n  allowed_hosts: [example.com]\n";
        fs::write(&path, entry.repeat(2)).unwrap();

        let result = SecretStore::new(&path).load();

        assert!(matches!(result, Err(SecretError::Duplicate { .. })));
    }

    #[test]
    fn load_rejects_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.yml");
        fs::write(&path, "not: a list").unwrap();

        let result = SecretStore::new(&path).load();

        assert!(matches!(result, Err(SecretError::Parse { .. })));
    }

    #[test]
    fn protect_database_makes_directory_private() {
        let home = tempfile::tempdir().unwrap();
        fs::create_dir(home.path().join("db")).unwrap();
        fs::set_permissions(home.path().join("db"), fs::Permissions::from_mode(0o755)).unwrap();

        protect_database(home.path()).unwrap();

        assert_eq!(mode(&home.path().join("db")), 0o700);
    }
}
