use reqwest::header::{HeaderMap, CONTENT_TYPE};
use serde_json::json;
use std::time::Duration;

use super::Result;
use crate::config::ModelConfig;
use crate::history::History;
use crate::tools::McpTool;

pub struct GeminiModel {
    client: reqwest::Client,
    config: ModelConfig,
}
