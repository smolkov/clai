//! The box of `/lsp`: the language servers of the clai server, their
//! state and last error lines; `r` restarts, `k` stops (RFC 0019).

use std::path::Path;

use crate::protocol::LspServerInfo;

/// Lines of a server's standard error shown under the list.
pub const STDERR_SHOWN: usize = 6;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LspBox {
    /// `None` until the server answers.
    pub servers: Option<Vec<LspServerInfo>>,
    pub selected: usize,
    /// The last restart or stop that failed.
    pub error: Option<String>,
}

impl LspBox {
    pub fn set(&mut self, servers: Vec<LspServerInfo>) {
        self.selected = self.selected.min(servers.len().saturating_sub(1));
        self.servers = Some(servers);
    }

    pub fn selected(&self) -> Option<&LspServerInfo> {
        self.servers.as_ref()?.get(self.selected)
    }

    /// Moves the selection by `by`, stopping at the ends.
    pub fn step(&mut self, by: isize) {
        let len = self.servers.as_ref().map_or(0, Vec::len);
        let at = self.selected.saturating_add_signed(by);
        self.selected = at.min(len.saturating_sub(1));
    }
}

/// A root as shown: relative to the session's workspace when inside it,
/// else absolute.
pub fn shown_root(root: &Path, workspace: &Path) -> String {
    match root.strip_prefix(workspace) {
        Ok(rest) if rest.as_os_str().is_empty() => ".".to_owned(),
        Ok(rest) => rest.display().to_string(),
        Err(_) => root.display().to_string(),
    }
}
