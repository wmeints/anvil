use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use std::result::Result;
use thiserror::Error;

/// Errors that can occur while loading a sandbox spec.
#[derive(Error, Debug)]
pub enum SandboxSpecError {
    #[error("can't find the source file")]
    FileNotFound,
    #[error("can't read contents of the input file")]
    CantReadInputFile(#[from] std::io::Error),
    #[error("can't parse input yaml")]
    InvalidSpec(#[from] serde_yaml::Error),
}

/// Describes a sandbox as configured in a spec file.
#[derive(Serialize, Deserialize, Debug)]
pub struct SandboxSpec {
    pub name: String,
    pub resources: Option<SandboxResourcesSpec>,
    pub image: Option<String>,
}

/// CPU and memory resources assigned to a sandbox.
#[derive(Serialize, Deserialize, Debug)]
pub struct SandboxResourcesSpec {
    pub cpu: u8,
    pub memory: String,
}

/// Loads and parses a sandbox spec from a YAML file.
pub fn from_file(path: &Path) -> Result<SandboxSpec, SandboxSpecError> {
    if !path.exists() {
        return Err(SandboxSpecError::FileNotFound);
    }

    let file_content =
        fs::read_to_string(path).map_err(|err| SandboxSpecError::CantReadInputFile(err))?;

    let spec = serde_yaml::from_str::<SandboxSpec>(file_content.as_str())
        .map_err(|err| SandboxSpecError::InvalidSpec(err))?;

    Ok(spec)
}

/// Creates a spec with the given name and default image and resources.
pub fn default_spec(name: String) -> SandboxSpec {
    SandboxSpec {
        name: name,
        image: Some("ubuntu:26.04".to_string()),
        resources: Some(SandboxResourcesSpec {
            cpu: 1,
            memory: "8 GiB".to_string(),
        }),
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
}
