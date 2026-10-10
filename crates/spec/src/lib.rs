use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;
use std::{fs, path::Path};

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::result::Result;
use thiserror::Error;

/// Errors that can occur while loading a sandbox spec.
#[derive(Error, Debug)]
pub enum SandboxSpecError {
    #[error("can't find the source file")]
    FileNotFound,
    #[error("can't read contents of the input file")]
    CantReadInputFile(#[from] std::io::Error),
    #[error("can't parse input yaml: {0}")]
    InvalidSpec(#[from] serde_yaml::Error),
    #[error("can't serialize the spec")]
    CantSerialize(#[source] serde_yaml::Error),
    #[error("can't write the spec file")]
    CantWriteFile(#[source] std::io::Error),
}

/// Errors that can occur while parsing a memory or disk size.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum SizeError {
    #[error(
        "invalid size `{0}`, expected a positive number with a unit, such as `512 MiB` or `4Gi`"
    )]
    Invalid(String),
    #[error("size `{0}` is too large")]
    TooLarge(String),
}

/// Image a sandbox runs when its spec doesn't name one: the `firebrick-base` image that the
/// release workflow publishes with the same version as this crate.
pub const DEFAULT_IMAGE: &str = concat!(
    "ghcr.io/wmeints/firebrick-base:v",
    env!("CARGO_PKG_VERSION")
);

/// A problem in a spec file, pinned to the 1-based line and column it occurs at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpecDiagnostic {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl SandboxSpecError {
    /// Returns the position and description of the problem when the spec content is invalid.
    pub fn diagnostic(&self) -> Option<SpecDiagnostic> {
        let SandboxSpecError::InvalidSpec(err) = self else {
            return None;
        };

        // Errors without a location concern the document as a whole.
        let (line, column) = err
            .location()
            .map_or((1, 1), |location| (location.line(), location.column()));

        // The yaml error message ends with the position, which we report separately.
        let message = err.to_string();
        let suffix = format!(" at line {line} column {column}");
        let message = message
            .strip_suffix(&suffix)
            .unwrap_or(&message)
            .to_string();

        Some(SpecDiagnostic {
            line,
            column,
            message,
        })
    }
}

/// Describes a sandbox as configured in a spec file.
#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct SandboxSpec {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<SandboxResourcesSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// Whether the sandbox runs the image's `/sbin/init` as PID 1. Defaults to `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init: Option<bool>,
    /// Whether fbkd trusts and installs the workspace's mise tools when the sandbox starts.
    /// Defaults to `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mise: Option<bool>,
    /// Sizes of the volumes the sandbox gets. Missing fields use their defaults.
    #[serde(default)]
    pub volumes: VolumesSpec,
    /// Egress rules of the sandbox. Without it, the sandbox gets microsandbox's default policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkSpec>,
}

/// CPU and memory resources assigned to a sandbox.
#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct SandboxResourcesSpec {
    pub cpu: u8,
    /// Memory size with a binary unit, such as `512 MiB` or `4Gi`.
    #[serde(deserialize_with = "deserialize_size")]
    pub memory: String,
}

impl Default for SandboxResourcesSpec {
    /// Returns the resources a sandbox gets when its spec doesn't set them.
    fn default() -> Self {
        Self {
            cpu: 2,
            memory: "4 GiB".to_string(),
        }
    }
}

/// Sizes of the volumes `fbkd` attaches to a sandbox, with binary units such as `20 GiB`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VolumesSpec {
    /// Size of the disk mounted at `/var/lib/docker`.
    #[serde(
        default = "default_docker_volume",
        deserialize_with = "deserialize_size"
    )]
    pub docker: String,
}

impl Default for VolumesSpec {
    /// Returns the volume sizes a sandbox gets when its spec doesn't set them.
    fn default() -> Self {
        Self {
            docker: default_docker_volume(),
        }
    }
}

/// Egress rules of a sandbox. The rules only apply when `enforce` is `true`, but they are
/// always validated.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NetworkSpec {
    /// Whether outgoing traffic is denied unless a rule allows it. Defaults to `false`.
    #[serde(default)]
    pub enforce: bool,
    /// Destinations the sandbox may connect to.
    #[serde(default)]
    pub allow: Vec<NetworkRule>,
    /// Destinations the sandbox may not connect to, even when an `allow` rule matches them.
    #[serde(default)]
    pub deny: Vec<NetworkRule>,
}

impl NetworkSpec {
    /// Adds the rules to `allow` that aren't in it yet, and removes them from `deny`. Returns
    /// whether the rules changed.
    pub fn allow(&mut self, rules: &[NetworkRule]) -> bool {
        move_rules(rules, &mut self.allow, &mut self.deny)
    }

    /// Adds the rules to `deny` that aren't in it yet, and removes them from `allow`. Returns
    /// whether the rules changed.
    pub fn deny(&mut self, rules: &[NetworkRule]) -> bool {
        move_rules(rules, &mut self.deny, &mut self.allow)
    }
}

/// Adds the rules to `to` that aren't in it yet and removes them from `from`. Returns whether
/// either list changed.
fn move_rules(
    rules: &[NetworkRule],
    to: &mut Vec<NetworkRule>,
    from: &mut Vec<NetworkRule>,
) -> bool {
    let before = from.len();
    from.retain(|rule| !rules.contains(rule));
    let mut changed = from.len() != before;

    for rule in rules {
        if !to.contains(rule) {
            to.push(rule.clone());
            changed = true;
        }
    }

    changed
}

/// The destination of a network rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkRule {
    /// Exactly this host name, such as `github.com`.
    Domain(String),
    /// The domain and every subdomain, written as `*.example.com`.
    DomainSuffix(String),
    /// A single IPv4 or IPv6 address.
    Ip(IpAddr),
    /// A CIDR range, such as `192.168.10.0/24`.
    Cidr {
        /// Address of the range.
        address: IpAddr,
        /// Number of leading bits of the address that the range fixes.
        prefix: u8,
    },
}

/// A network rule that isn't a host name, `*.domain`, IP address or CIDR range.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("invalid network rule \"{0}\": use a host name, *.domain, an IP address or a CIDR range")]
pub struct InvalidNetworkRule(pub String);

impl FromStr for NetworkRule {
    type Err = InvalidNetworkRule;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        parse_rule(value).ok_or_else(|| InvalidNetworkRule(value.to_string()))
    }
}

impl fmt::Display for NetworkRule {
    /// Writes the rule the way it is written in the spec.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkRule::Domain(domain) => f.write_str(domain),
            NetworkRule::DomainSuffix(domain) => write!(f, "*.{domain}"),
            NetworkRule::Ip(address) => write!(f, "{address}"),
            NetworkRule::Cidr { address, prefix } => write!(f, "{address}/{prefix}"),
        }
    }
}

impl Serialize for NetworkRule {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for NetworkRule {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_str(NetworkRuleVisitor)
    }
}

/// Parses the rule while the parser still points at it, so a problem is reported at the
/// rule's line and column.
struct NetworkRuleVisitor;

impl Visitor<'_> for NetworkRuleVisitor {
    type Value = NetworkRule;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a host name, *.domain, an IP address or a CIDR range")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<NetworkRule, E> {
        value.parse().map_err(E::custom)
    }
}

/// Parses a rule, or returns `None` when it has none of the supported forms.
fn parse_rule(value: &str) -> Option<NetworkRule> {
    if let Some(domain) = value.strip_prefix("*.") {
        // A single label such as `*.com` would match a whole top-level domain.
        return (is_host_name(domain) && domain.contains('.'))
            .then(|| NetworkRule::DomainSuffix(domain.to_string()));
    }

    if let Ok(address) = value.parse() {
        return Some(NetworkRule::Ip(address));
    }

    if let Some((address, prefix)) = value.split_once('/') {
        return parse_cidr(address, prefix);
    }

    is_host_name(value).then(|| NetworkRule::Domain(value.to_string()))
}

/// Parses the address and prefix length of a CIDR range.
fn parse_cidr(address: &str, prefix: &str) -> Option<NetworkRule> {
    let address: IpAddr = address.parse().ok()?;
    let prefix: u8 = prefix.parse().ok()?;
    let max_prefix = if address.is_ipv4() { 32 } else { 128 };

    (prefix <= max_prefix).then_some(NetworkRule::Cidr { address, prefix })
}

/// Whether the value is a DNS host name: dot-separated labels of letters, digits and hyphens.
fn is_host_name(value: &str) -> bool {
    value.len() <= 253 && value.split('.').all(is_host_label)
}

/// Whether the value is one label of a host name, such as `github` in `github.com`.
fn is_host_label(label: &str) -> bool {
    (1..=63).contains(&label.len())
        && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

/// Size of the Docker volume when the spec doesn't set one.
fn default_docker_volume() -> String {
    "20 GiB".to_string()
}

/// Parses a memory or disk size such as `512 MiB`, `512Mi`, `4 GiB` or `4Gi` into mebibytes.
pub fn parse_size_mib(value: &str) -> Result<u32, SizeError> {
    let invalid = || SizeError::Invalid(value.to_string());

    let trimmed = value.trim();
    let digits_end = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (amount, unit) = trimmed.split_at(digits_end);

    let amount: u64 = amount.parse().map_err(|_| invalid())?;
    let multiplier: u64 = match unit.trim_start() {
        "Mi" | "MiB" => 1,
        "Gi" | "GiB" => 1024,
        _ => return Err(invalid()),
    };

    if amount == 0 {
        return Err(invalid());
    }

    amount
        .checked_mul(multiplier)
        .and_then(|mib| u32::try_from(mib).ok())
        .ok_or_else(|| SizeError::TooLarge(value.to_string()))
}

/// Deserializes a size, rejecting values that `parse_size_mib` can't read.
fn deserialize_size<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    deserializer.deserialize_str(SizeVisitor)
}

/// Checks the size while the parser still points at the value, so a problem is
/// reported at the value's line and column instead of at the enclosing mapping.
struct SizeVisitor;

impl Visitor<'_> for SizeVisitor {
    type Value = String;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a size such as `512 MiB` or `4Gi`")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
        parse_size_mib(value).map_err(E::custom)?;

        Ok(value.to_string())
    }
}

/// Loads and parses a sandbox spec from a YAML file.
pub fn from_file(path: &Path) -> Result<SandboxSpec, SandboxSpecError> {
    if !path.exists() {
        return Err(SandboxSpecError::FileNotFound);
    }

    let file_content = fs::read_to_string(path).map_err(SandboxSpecError::CantReadInputFile)?;

    let spec = serde_yaml::from_str::<SandboxSpec>(file_content.as_str())
        .map_err(SandboxSpecError::InvalidSpec)?;

    Ok(spec)
}

/// Writes the spec to a YAML file, replacing the file's contents. Comments and formatting of an
/// existing file are lost, and fields that aren't set are left out.
pub fn to_file(spec: &SandboxSpec, path: &Path) -> Result<(), SandboxSpecError> {
    let content = serde_yaml::to_string(spec).map_err(SandboxSpecError::CantSerialize)?;

    fs::write(path, content).map_err(SandboxSpecError::CantWriteFile)
}

/// Creates a spec with the given name and default image and resources.
pub fn default_spec(name: String) -> SandboxSpec {
    SandboxSpec {
        name,
        image: Some(DEFAULT_IMAGE.to_string()),
        resources: Some(SandboxResourcesSpec::default()),
        init: Some(true),
        mise: Some(true),
        volumes: VolumesSpec::default(),
        network: None,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    fn write_spec(content: &str) -> NamedTempFile {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(content.as_bytes()).unwrap();
        file
    }

    #[test]
    fn parses_full_spec() {
        let file =
            write_spec("name: dev\nresources:\n  cpu: 4\n  memory: 8Gi\nimage: ubuntu:24.04\n");

        let spec = from_file(file.path()).unwrap();
        let resources = spec.resources.unwrap();

        assert_eq!(spec.name, "dev");
        assert_eq!(resources.cpu, 4);
        assert_eq!(resources.memory, "8Gi");
        assert_eq!(spec.image.as_deref(), Some("ubuntu:24.04"));
    }

    #[test]
    fn image_is_optional() {
        let file = write_spec("name: dev\nresources:\n  cpu: 2\n  memory: 4Gi\n");

        let spec = from_file(file.path()).unwrap();

        assert!(spec.image.is_none());
    }

    #[test]
    fn missing_file_returns_file_not_found() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("does-not-exist.yaml");

        let result = from_file(&path);

        assert!(matches!(result, Err(SandboxSpecError::FileNotFound)));
    }

    #[test]
    fn directory_returns_cant_read_input_file() {
        let dir = TempDir::new().unwrap();

        let result = from_file(dir.path());

        assert!(matches!(
            result,
            Err(SandboxSpecError::CantReadInputFile(_))
        ));
    }

    #[test]
    fn resources_are_optional() {
        let file = write_spec("name: dev\n");

        let spec = from_file(file.path()).unwrap();

        assert!(spec.resources.is_none());
    }

    #[test]
    fn missing_required_field_returns_invalid_spec() {
        let file = write_spec("image: ubuntu:24.04\n");

        let result = from_file(file.path());

        assert!(matches!(result, Err(SandboxSpecError::InvalidSpec(_))));
    }

    #[test]
    fn malformed_yaml_returns_invalid_spec() {
        let file = write_spec("name: [unclosed\n");

        let result = from_file(file.path());

        assert!(matches!(result, Err(SandboxSpecError::InvalidSpec(_))));
    }

    #[test]
    fn cpu_out_of_range_returns_invalid_spec() {
        let file = write_spec("name: dev\nresources:\n  cpu: 256\n  memory: 4Gi\n");

        let result = from_file(file.path());

        assert!(matches!(result, Err(SandboxSpecError::InvalidSpec(_))));
    }

    #[test]
    fn unknown_field_returns_invalid_spec() {
        let file = write_spec("name: dev\nimgae: ubuntu:24.04\n");

        let result = from_file(file.path());

        assert!(matches!(result, Err(SandboxSpecError::InvalidSpec(_))));
    }

    #[test]
    fn diagnostic_reports_position_of_invalid_value() {
        let file = write_spec("name: dev\nresources:\n  cpu: 256\n  memory: 4Gi\n");

        let diagnostic = from_file(file.path()).unwrap_err().diagnostic().unwrap();

        assert_eq!(diagnostic.line, 3);
        assert_eq!(diagnostic.column, 8);
        assert_eq!(
            diagnostic.message,
            "resources.cpu: invalid value: integer `256`, expected u8"
        );
    }

    #[test]
    fn diagnostic_reports_position_of_syntax_error() {
        let file = write_spec("name: dev\n  bad: indent\n");

        let diagnostic = from_file(file.path()).unwrap_err().diagnostic().unwrap();

        assert_eq!((diagnostic.line, diagnostic.column), (2, 6));
        assert!(!diagnostic.message.contains("at line"));
    }

    #[test]
    fn parses_sizes() {
        let cases = [
            ("512 MiB", 512),
            ("512Mi", 512),
            ("4 GiB", 4096),
            ("4Gi", 4096),
            (" 2GiB ", 2048),
        ];

        for (input, expected) in cases {
            assert_eq!(parse_size_mib(input), Ok(expected), "{input:?}");
        }
    }

    #[test]
    fn rejects_invalid_sizes() {
        for input in ["", "lots", "4", "4 GB", "0 GiB", "-1 GiB", "1.5 GiB", "GiB"] {
            assert!(
                matches!(parse_size_mib(input), Err(SizeError::Invalid(_))),
                "{input:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_sizes_that_overflow() {
        assert!(matches!(
            parse_size_mib("4194304 GiB"),
            Err(SizeError::TooLarge(_))
        ));
    }

    #[test]
    fn invalid_memory_returns_diagnostic() {
        let file = write_spec("name: dev\nresources:\n  cpu: 2\n  memory: lots\n");

        let diagnostic = from_file(file.path()).unwrap_err().diagnostic().unwrap();

        assert_eq!((diagnostic.line, diagnostic.column), (4, 11));
        assert!(
            diagnostic.message.contains("invalid size `lots`"),
            "{}",
            diagnostic.message
        );
    }

    #[test]
    fn volumes_are_optional() {
        let file = write_spec("name: dev\n");

        let spec = from_file(file.path()).unwrap();

        assert_eq!(spec.volumes, VolumesSpec::default());
        assert_eq!(spec.volumes.docker, "20 GiB");
    }

    #[test]
    fn docker_volume_is_optional() {
        let file = write_spec("name: dev\nvolumes: {}\n");

        let spec = from_file(file.path()).unwrap();

        assert_eq!(spec.volumes.docker, "20 GiB");
    }

    #[test]
    fn parses_docker_volume() {
        let file = write_spec(
            "name: dev\nresources:\n  cpu: 1\n  memory: 2GiB\nvolumes:\n  docker: 40GiB\n",
        );

        let spec = from_file(file.path()).unwrap();

        assert_eq!(spec.volumes.docker, "40GiB");
    }

    #[test]
    fn invalid_docker_volume_returns_diagnostic() {
        for size in ["20 GB", "0 GiB"] {
            let file = write_spec(&format!("name: dev\nvolumes:\n  docker: {size}\n"));

            let diagnostic = from_file(file.path()).unwrap_err().diagnostic().unwrap();

            assert_eq!((diagnostic.line, diagnostic.column), (3, 11), "{size:?}");
            assert!(
                diagnostic
                    .message
                    .starts_with(&format!("volumes.docker: invalid size `{size}`")),
                "{}",
                diagnostic.message
            );
        }
    }

    #[test]
    fn unknown_volume_returns_invalid_spec() {
        let file = write_spec("name: dev\nvolumes:\n  dokcer: 20GiB\n");

        let result = from_file(file.path());

        assert!(matches!(result, Err(SandboxSpecError::InvalidSpec(_))));
    }

    #[test]
    fn default_spec_uses_default_image_and_resources() {
        let spec = default_spec("dev".to_string());
        let resources = spec.resources.unwrap();

        assert_eq!(spec.image.as_deref(), Some(DEFAULT_IMAGE));
        assert_eq!((resources.cpu, resources.memory.as_str()), (2, "4 GiB"));
        assert_eq!(spec.init, Some(true));
        assert_eq!(spec.mise, Some(true));
        assert_eq!(spec.volumes, VolumesSpec::default());
        assert!(spec.network.is_none());
    }

    #[test]
    fn init_is_optional() {
        let file = write_spec("name: dev\n");

        let spec = from_file(file.path()).unwrap();

        assert!(spec.init.is_none());
    }

    #[test]
    fn parses_init() {
        let file = write_spec("name: dev\nimage: alpine:3.22\ninit: false\n");

        let spec = from_file(file.path()).unwrap();

        assert_eq!(spec.init, Some(false));
    }

    #[test]
    fn invalid_init_returns_diagnostic() {
        let file = write_spec("name: dev\ninit: sometimes\n");

        let diagnostic = from_file(file.path()).unwrap_err().diagnostic().unwrap();

        assert_eq!(diagnostic.line, 2);
        assert!(
            diagnostic.message.contains("init"),
            "{}",
            diagnostic.message
        );
    }

    #[test]
    fn mise_is_optional() {
        let file = write_spec("name: dev\n");

        let spec = from_file(file.path()).unwrap();

        assert!(spec.mise.is_none());
    }

    #[test]
    fn parses_mise() {
        let file = write_spec("name: dev\nmise: false\n");

        let spec = from_file(file.path()).unwrap();

        assert_eq!(spec.mise, Some(false));
    }

    #[test]
    fn invalid_mise_returns_diagnostic() {
        let file = write_spec("name: dev\nmise: sometimes\n");

        let diagnostic = from_file(file.path()).unwrap_err().diagnostic().unwrap();

        assert_eq!((diagnostic.line, diagnostic.column), (2, 7));
        assert!(
            diagnostic.message.contains("mise"),
            "{}",
            diagnostic.message
        );
    }

    #[test]
    fn network_is_optional() {
        let file = write_spec("name: dev\n");

        let spec = from_file(file.path()).unwrap();

        assert!(spec.network.is_none());
    }

    #[test]
    fn network_fields_have_defaults() {
        let file = write_spec("name: dev\nnetwork: {}\n");

        let spec = from_file(file.path()).unwrap();

        assert_eq!(spec.network, Some(NetworkSpec::default()));
        assert!(!NetworkSpec::default().enforce);
    }

    #[test]
    fn parses_network() {
        let file = write_spec(concat!(
            "name: dev\n",
            "network:\n",
            "  enforce: true\n",
            "  allow:\n",
            "    - github.com\n",
            "    - \"*.githubusercontent.com\"\n",
            "  deny:\n",
            "    - gist.github.com\n",
        ));

        let network = from_file(file.path()).unwrap().network.unwrap();

        assert!(network.enforce);
        assert_eq!(
            network.allow,
            [
                NetworkRule::Domain("github.com".to_string()),
                NetworkRule::DomainSuffix("githubusercontent.com".to_string()),
            ]
        );
        assert_eq!(
            network.deny,
            [NetworkRule::Domain("gist.github.com".to_string())]
        );
    }

    #[test]
    fn parses_addresses_and_ranges() {
        let ip = |value: &str| value.parse::<IpAddr>().unwrap();
        let cases = [
            ("140.82.112.4", NetworkRule::Ip(ip("140.82.112.4"))),
            ("2001:db8::1", NetworkRule::Ip(ip("2001:db8::1"))),
            (
                "192.168.10.0/24",
                NetworkRule::Cidr {
                    address: ip("192.168.10.0"),
                    prefix: 24,
                },
            ),
            (
                "2001:db8::/32",
                NetworkRule::Cidr {
                    address: ip("2001:db8::"),
                    prefix: 32,
                },
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(input.parse(), Ok(expected), "{input:?}");
        }
    }

    #[test]
    fn network_rule_displays_as_written() {
        for rule in [
            "github.com",
            "*.example.com",
            "10.0.0.1",
            "10.0.0.0/8",
            "::1",
            "fd00::/8",
        ] {
            assert_eq!(rule.parse::<NetworkRule>().unwrap().to_string(), rule);
        }
    }

    #[test]
    fn rejects_invalid_network_rules() {
        for rule in [
            "",
            "*",
            "*.",
            "*.com",
            "foo.*.com",
            "github.*",
            "https://github.com",
            "github.com:443",
            "github.com/path",
            "10.0.0.0/33",
            "::/129",
            "10.0.0/8",
            "10.0.0.0/",
            "-github.com",
            "github..com",
            "git hub.com",
        ] {
            assert_eq!(
                rule.parse::<NetworkRule>(),
                Err(InvalidNetworkRule(rule.to_string())),
                "{rule:?} should be rejected"
            );
        }
    }

    #[test]
    fn invalid_network_rule_returns_diagnostic() {
        let file = write_spec(
            "name: dev\nnetwork:\n  allow:\n    - github.com\n    - https://github.com\n",
        );

        let diagnostic = from_file(file.path()).unwrap_err().diagnostic().unwrap();

        assert_eq!((diagnostic.line, diagnostic.column), (5, 7));
        assert!(
            diagnostic.message.ends_with(
                "invalid network rule \"https://github.com\": use a host name, *.domain, \
                 an IP address or a CIDR range"
            ),
            "{}",
            diagnostic.message
        );
    }

    #[test]
    fn unknown_network_field_returns_invalid_spec() {
        let file = write_spec("name: dev\nnetwork:\n  enfroce: true\n");

        let result = from_file(file.path());

        assert!(matches!(result, Err(SandboxSpecError::InvalidSpec(_))));
    }

    #[test]
    fn missing_file_has_no_diagnostic() {
        let dir = TempDir::new().unwrap();

        let result = from_file(&dir.path().join("does-not-exist.yaml"));

        assert!(result.unwrap_err().diagnostic().is_none());
    }

    fn rules(values: &[&str]) -> Vec<NetworkRule> {
        values.iter().map(|value| value.parse().unwrap()).collect()
    }

    fn network(allow: &[&str], deny: &[&str]) -> NetworkSpec {
        NetworkSpec {
            enforce: false,
            allow: rules(allow),
            deny: rules(deny),
        }
    }

    #[test]
    fn allow_adds_rules_and_removes_them_from_deny() {
        let mut spec = network(&["github.com"], &["example.org", "10.0.0.0/8"]);

        assert!(spec.allow(&rules(&["example.org", "*.npmjs.org"])));

        assert_eq!(
            spec,
            network(
                &["github.com", "example.org", "*.npmjs.org"],
                &["10.0.0.0/8"]
            )
        );
    }

    #[test]
    fn deny_adds_rules_and_removes_them_from_allow() {
        let mut spec = network(&["github.com", "1.1.1.1"], &[]);

        assert!(spec.deny(&rules(&["1.1.1.1"])));

        assert_eq!(spec, network(&["github.com"], &["1.1.1.1"]));
    }

    #[test]
    fn allow_skips_rules_that_are_already_allowed() {
        let mut spec = network(&["github.com"], &[]);

        assert!(!spec.allow(&rules(&["github.com"])));
        assert!(spec.allow(&rules(&["example.org", "example.org"])));

        assert_eq!(spec, network(&["github.com", "example.org"], &[]));
    }

    #[test]
    fn deny_reports_no_change_for_rules_that_are_already_denied() {
        let mut spec = network(&[], &["example.org"]);

        assert!(!spec.deny(&rules(&["example.org"])));
        assert_eq!(spec, network(&[], &["example.org"]));
    }

    #[test]
    fn to_file_round_trips_the_spec() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".firebrick.yml");
        let mut spec = default_spec("dev".to_string());
        spec.network = Some(NetworkSpec {
            enforce: true,
            ..network(
                &["github.com", "*.npmjs.org", "10.0.0.0/8"],
                &["2001:db8::1"],
            )
        });

        to_file(&spec, &path).unwrap();
        let loaded = from_file(&path).unwrap();

        assert_eq!(loaded.name, "dev");
        assert_eq!(loaded.image, spec.image);
        assert_eq!(loaded.init, Some(true));
        assert_eq!(loaded.mise, Some(true));
        assert_eq!(loaded.volumes, spec.volumes);
        assert_eq!(loaded.network, spec.network);
    }

    #[test]
    fn to_file_leaves_out_unset_fields() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".firebrick.yml");
        let spec = from_file(write_spec("name: dev\n").path()).unwrap();

        to_file(&spec, &path).unwrap();
        let content = fs::read_to_string(&path).unwrap();

        assert!(!content.contains("null"), "{content}");
        assert!(!content.contains("network"), "{content}");
    }

    #[test]
    fn to_file_reports_write_failure() {
        let dir = TempDir::new().unwrap();

        let result = to_file(&default_spec("dev".to_string()), dir.path());

        assert!(matches!(result, Err(SandboxSpecError::CantWriteFile(_))));
    }
}
