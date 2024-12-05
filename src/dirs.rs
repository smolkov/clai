use std::path::PathBuf;

use once_cell::sync::Lazy;

// paths and directories
pub static HOME: Lazy<PathBuf> = Lazy::new(|| dirs::home_dir().unwrap_or(PathBuf::from("/")));

pub static CONFIGS: Lazy<PathBuf> = Lazy::new(|| dirs::config_dir().unwrap_or(PathBuf::from(".")));
pub static CONFIG_DIR: Lazy<PathBuf> = Lazy::new(|| CONFIGS.join("clai"));
pub static CONFIG_FILE: Lazy<PathBuf> = Lazy::new(|| CONFIG_DIR.join("config.toml"));
