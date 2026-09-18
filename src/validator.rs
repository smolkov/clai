use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Validator {
    root: PathBuf,
}

impl Validator {
    pub fn new<T: AsRef<Path>>(root: T) -> Self {
        Validator {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn validate_path(&self, path: &str) -> Result<PathBuf> {
        let requested = self.root.join(path);

        // Prevents ../../ directly in the string before anything exists
        if requested
            .components()
            .any(|c| c == std::path::Component::ParentDir)
        {
            return Err(anyhow::anyhow!(
                "Invalid path: contains parent directory reference"
            ));
        }

        // Canonicalize the requested path to resolve symlinks and relative components
        let canonical_requested = match requested.canonicalize() {
            Ok(p) => p,
            Err(_) => return Err(anyhow::anyhow!("Invalid path: cannot canonicalize")),
        };

        // Check if the canonicalized path starts with the root directory
        if !canonical_requested.starts_with(&self.root) {
            return Err(anyhow::anyhow!("Invalid path: outside of allowed root"));
        }
        Ok(canonical_requested)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}
