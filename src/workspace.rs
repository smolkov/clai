use anyhow::Result;
use once_cell::sync::Lazy;
use std::fs;
use std::path::{Path, PathBuf};
pub const CONFIG_FILE: &str = "config.toml";
pub const CLAI_DIR: &str = "clai";

// Solution 2 all dirs in struct
pub static WORKSPACE: Lazy<Workspace> = Lazy::new(Workspace::new);

#[derive(Debug)]
pub struct Workspace {
    home: PathBuf,
    config: PathBuf,
    config_file: PathBuf,
    local_config_file: PathBuf,
    allowed_root: PathBuf,
}

impl Workspace {
    pub fn new() -> Workspace {
        let home = dirs::home_dir().unwrap_or(PathBuf::from("."));
        let config = dirs::config_dir().unwrap_or(PathBuf::from("."));
        let config = config.join(CLAI_DIR);
        if !config.is_dir() {
            fs::create_dir_all(&config).expect(
                "Error initializing workspace: Failed to create configuration directory '.sprsh'",
            );
        }
        let config_file = config.join(CONFIG_FILE);
        let local_config_file = PathBuf::from(&format!("./{CONFIG_FILE}"));
        Workspace {
            home,
            config,
            config_file,
            local_config_file,
            allowed_root: PathBuf::from(".")
                .canonicalize()
                .expect("Failed to canonicalize current directory"),
        }
    }
    pub fn home(&self) -> &Path {
        &self.home
    }
    pub fn config(&self) -> &Path {
        &self.config
    }
    pub fn config_file(&self) -> &Path {
        &self.config_file
    }
    pub fn local_config_file(&self) -> &Path {
        &self.local_config_file
    }
    pub fn allowed_root(&self) -> &Path {
        &self.allowed_root
    }
    pub fn validate_path(&self, path: &str) -> Result<PathBuf> {
        let requested = self.allowed_root.join(path);

        // Verhindert ../../ direkt im String, bevor überhaupt was existiert
        if path.contains("..") {
            return Err(anyhow::anyhow!(
                "Path {path} is not allowed. Must be within {:?}",
                self.allowed_root
            ));
        }
        let mut check_dir = requested
            .parent()
            .unwrap_or(&self.allowed_root)
            .to_path_buf();

        while !check_dir.exists() {
            check_dir = check_dir
                .parent()
                .ok_or(anyhow::anyhow!(
                    "Path {path} is not allowed. Must be within {:?}",
                    self.allowed_root
                ))?
                .to_path_buf();
        }
        let canonical_parent = check_dir.canonicalize().map_err(|_| anyhow::anyhow!(
            "Path {path} is not allowed. Must be within {:?}",
            self.allowed_root
        ))?;

        if !canonical_parent.starts_with(&self.allowed_root) {
            return Err(anyhow::anyhow!(
                "Path {path} is not allowed. Must be within {:?}",
                self.allowed_root
            ));
        }
        Ok(requested)
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace::new()
    }
}
