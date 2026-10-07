//! Secrets that sandboxes use without seeing their values.

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use std::io::Read;
use tonic::transport::Channel;

use crate::api::sandbox_management_service_client::SandboxManagementServiceClient;
use crate::api::{ListSecretsRequest, RemoveSecretRequest, SecretSummary, SetSecretRequest};
use crate::manage::OutputFormat;
use crate::table;

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

/// Prints the secrets' names and allowed hosts in the given format. Values never leave the
/// daemon.
pub async fn list(
    format: OutputFormat,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let secrets = client
        .list_secrets(ListSecretsRequest {})
        .await
        .map_err(|status| anyhow!("{}", status.message()))?
        .into_inner()
        .secrets;

    let output = match format {
        OutputFormat::Table => render_table(&secrets)?,
        OutputFormat::Json => render_json(&secrets)?,
    };

    print!("{output}");

    Ok(())
}

/// Removes a secret from all sandboxes and from the daemon. When a sandbox fails, the daemon
/// keeps the secret so the removal can be retried.
pub async fn remove(
    name: String,
    client: &mut SandboxManagementServiceClient<Channel>,
) -> Result<()> {
    let response = client
        .remove_secret(RemoveSecretRequest { name: name.clone() })
        .await
        .map_err(|status| anyhow!("{}", status.message()))?
        .into_inner();

    if !response.failed_sandboxes.is_empty() {
        bail!(
            "couldn't remove secret {name} from sandboxes {}, see the anvild log for details. \
             The secret is kept, so run `anvil secret rm {name}` again to retry",
            response.failed_sandboxes.join(", ")
        );
    }

    println!("Secret {name} removed. Running sandboxes keep using it until they restart.");

    Ok(())
}

/// Renders the secrets as a table with their name and allowed hosts.
fn render_table(secrets: &[SecretSummary]) -> Result<String> {
    let rows: Vec<[String; 2]> = secrets
        .iter()
        .map(|secret| [secret.name.clone(), secret.allowed_hosts.join(", ")])
        .collect();

    table::render(["NAME", "ALLOWED HOSTS"], &rows)
}

/// A secret as printed in the JSON output.
#[derive(Serialize)]
struct SecretListing<'a> {
    name: &'a str,
    allowed_hosts: &'a [String],
}

/// Renders the secrets as a pretty-printed JSON array.
fn render_json(secrets: &[SecretSummary]) -> Result<String> {
    let listings: Vec<SecretListing> = secrets
        .iter()
        .map(|secret| SecretListing {
            name: &secret.name,
            allowed_hosts: &secret.allowed_hosts,
        })
        .collect();

    Ok(format!("{}\n", serde_json::to_string_pretty(&listings)?))
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

    fn summary(name: &str, hosts: &[&str]) -> SecretSummary {
        SecretSummary {
            name: name.to_string(),
            allowed_hosts: hosts.iter().map(|host| host.to_string()).collect(),
        }
    }

    #[test]
    fn table_lists_names_and_hosts() {
        let secrets = [summary("GH_TOKEN", &["github.com", "api.github.com"])];

        let table = render_table(&secrets).unwrap();

        assert_eq!(
            table,
            "┌──────────┬────────────────────────────┐\n\
             │ NAME     │ ALLOWED HOSTS              │\n\
             ├──────────┼────────────────────────────┤\n\
             │ GH_TOKEN │ github.com, api.github.com │\n\
             └──────────┴────────────────────────────┘\n"
        );
    }

    #[test]
    fn empty_table_shows_header() {
        let table = render_table(&[]).unwrap();

        assert!(table.contains("NAME"), "{table}");
        assert!(table.contains("ALLOWED HOSTS"), "{table}");
    }

    #[test]
    fn json_lists_names_and_hosts() {
        let secrets = [summary("A", &["example.com"])];

        let json: serde_json::Value =
            serde_json::from_str(&render_json(&secrets).unwrap()).unwrap();

        assert_eq!(
            json,
            serde_json::json!([{ "name": "A", "allowed_hosts": ["example.com"] }])
        );
    }

    #[test]
    fn empty_json_is_empty_array() {
        assert_eq!(render_json(&[]).unwrap(), "[]\n");
    }

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
