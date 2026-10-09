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
}

/// Errors that can occur while parsing a memory size.
#[derive(Error, Debug, PartialEq, Eq)]
pub enum MemorySizeError {
    #[error(
        "invalid memory size `{0}`, expected a positive number with a unit, such as `512 MiB` or `4Gi`"
    )]
    Invalid(String),
    #[error("memory size `{0}` is too large")]
    TooLarge(String),
}

/// Image a sandbox runs when its spec doesn't name one: the `anvil-base` image that the
/// release workflow publishes with the same version as this crate.
pub const DEFAULT_IMAGE: &str = concat!("ghcr.io/wmeints/anvil-base:v", env!("CARGO_PKG_VERSION"));

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
    pub resources: Option<SandboxResourcesSpec>,
    pub image: Option<String>,
    /// Whether the sandbox runs the image's `/sbin/init` as PID 1. Defaults to `true`.
    pub init: Option<bool>,
}

/// CPU and memory resources assigned to a sandbox.
#[derive(Serialize, Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct SandboxResourcesSpec {
    pub cpu: u8,
    /// Memory size with a binary unit, such as `512 MiB` or `4Gi`.
    #[serde(deserialize_with = "deserialize_memory")]
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

/// Parses a memory size such as `512 MiB`, `512Mi`, `4 GiB` or `4Gi` into mebibytes.
pub fn parse_memory_mib(value: &str) -> Result<u32, MemorySizeError> {
    let invalid = || MemorySizeError::Invalid(value.to_string());

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
        .ok_or_else(|| MemorySizeError::TooLarge(value.to_string()))
}

/// Deserializes a memory size, rejecting values that `parse_memory_mib` can't read.
fn deserialize_memory<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    deserializer.deserialize_str(MemoryVisitor)
}

/// Checks the memory size while the parser still points at the value, so a problem is
/// reported at the value's line and column instead of at the enclosing mapping.
struct MemoryVisitor;

impl Visitor<'_> for MemoryVisitor {
    type Value = String;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a memory size such as `512 MiB` or `4Gi`")
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<String, E> {
        parse_memory_mib(value).map_err(E::custom)?;

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

/// Creates a spec with the given name and default image and resources.
pub fn default_spec(name: String) -> SandboxSpec {
    SandboxSpec {
        name,
        image: Some(DEFAULT_IMAGE.to_string()),
        resources: Some(SandboxResourcesSpec::default()),
        init: Some(true),
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
    fn parses_memory_sizes() {
        let cases = [
            ("512 MiB", 512),
            ("512Mi", 512),
            ("4 GiB", 4096),
            ("4Gi", 4096),
            (" 2GiB ", 2048),
        ];

        for (input, expected) in cases {
            assert_eq!(parse_memory_mib(input), Ok(expected), "{input:?}");
        }
    }

    #[test]
    fn rejects_invalid_memory_sizes() {
        for input in ["", "lots", "4", "4 GB", "0 GiB", "-1 GiB", "1.5 GiB", "GiB"] {
            assert!(
                matches!(parse_memory_mib(input), Err(MemorySizeError::Invalid(_))),
                "{input:?} should be rejected"
            );
        }
    }

    #[test]
    fn rejects_memory_sizes_that_overflow() {
        assert!(matches!(
            parse_memory_mib("4194304 GiB"),
            Err(MemorySizeError::TooLarge(_))
        ));
    }

    #[test]
    fn invalid_memory_returns_diagnostic() {
        let file = write_spec("name: dev\nresources:\n  cpu: 2\n  memory: lots\n");

        let diagnostic = from_file(file.path()).unwrap_err().diagnostic().unwrap();

        assert_eq!((diagnostic.line, diagnostic.column), (4, 11));
        assert!(diagnostic.message.contains("invalid memory size `lots`"));
    }

    #[test]
    fn default_spec_uses_default_image_and_resources() {
        let spec = default_spec("dev".to_string());
        let resources = spec.resources.unwrap();

        assert_eq!(spec.image.as_deref(), Some(DEFAULT_IMAGE));
        assert_eq!((resources.cpu, resources.memory.as_str()), (2, "4 GiB"));
        assert_eq!(spec.init, Some(true));
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
    fn missing_file_has_no_diagnostic() {
        let dir = TempDir::new().unwrap();

        let result = from_file(&dir.path().join("does-not-exist.yaml"));

        assert!(result.unwrap_err().diagnostic().is_none());
    }
}
