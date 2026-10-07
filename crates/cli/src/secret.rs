//! Secrets that sandboxes use without seeing their values.

use anyhow::{Context, Result, anyhow};
use std::io::Read;
use tonic::transport::Channel;

use crate::api::SetSecretRequest;
use crate::api::sandbox_management_service_client::SandboxManagementServiceClient;

/// Stores a secret in the daemon, which adds it to all sandboxes.
pub async fn set(
    name: String,
    value: String,
    allowed_hosts: Vec<String>,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let response = client
        .set_secret(SetSecretRequest {
            name: name.clone(),
            value,
            allowed_hosts,
        })
        .await
        .map_err(|status| anyhow!("{}", status.message()))?
        .into_inner();

    println!(
        "Secret {name} set. Sandboxes see the placeholder $MSB_{name}; running sandboxes get it after a restart."
    );

    for sandbox in response.failed_sandboxes {
        eprintln!(
            "warning: couldn't add secret {name} to sandbox {sandbox}, see the anvild log for details"
        );
    }

    Ok(())
}

/// Reads a secret value from `reader`, without the line ending that `echo` and most password
/// managers add.
pub fn read_value(mut reader: impl Read) -> Result<String> {
    let mut value = String::new();
    reader
        .read_to_string(&mut value)
        .context("failed to read the secret value from stdin")?;

    let trimmed = value
        .strip_suffix("\r\n")
        .or_else(|| value.strip_suffix('\n'))
        .unwrap_or(&value);

    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_value_removes_one_line_ending() {
        assert_eq!(read_value("token\n".as_bytes()).unwrap(), "token");
        assert_eq!(read_value("token\r\n".as_bytes()).unwrap(), "token");
        assert_eq!(read_value("token\n\n".as_bytes()).unwrap(), "token\n");
    }

    #[test]
    fn read_value_keeps_other_whitespace() {
        assert_eq!(read_value(" to ken ".as_bytes()).unwrap(), " to ken ");
        assert_eq!(read_value("".as_bytes()).unwrap(), "");
    }

    #[test]
    fn read_value_rejects_invalid_utf8() {
        assert!(read_value(&[0xff, 0xfe][..]).is_err());
    }
}
