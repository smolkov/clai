//! The state of the model picker (see `spec/tui.md`, "Model picker"): the
//! models from the server, the filter and the selected row. `App` routes keys
//! here; `view::picker` draws it.

use std::path::PathBuf;

use crate::protocol::ModelInfo;

use super::editor::{Edit, Editor};

/// What the chosen model is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Purpose {
    /// The attached session's model.
    Change,
    /// A new session, created when a model is chosen.
    New {
        workspace: PathBuf,
        name: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Picker {
    pub purpose: Purpose,
    models: Vec<ModelInfo>,
    /// The user config file, for the text shown when there are no models.
    pub config: PathBuf,
    pub filter: Editor,
    /// Among the visible models.
    selected: usize,
}

impl Picker {
    /// Starts on the current model; for a new session on `default_model`.
    pub fn new(models: Vec<ModelInfo>, config: PathBuf, current: &str, purpose: Purpose) -> Picker {
        let start = |m: &ModelInfo| match purpose {
            Purpose::Change => m.model == current,
            Purpose::New { .. } => m.default,
        };
        let selected = models.iter().position(start).unwrap_or(0);
        Picker {
            purpose,
            models,
            config,
            filter: Editor::default(),
            selected,
        }
    }

    /// A newer list, after a reload: the same model stays selected if it can.
    pub fn set_models(&mut self, models: Vec<ModelInfo>, config: PathBuf) {
        let chosen = self.chosen().map(|m| m.model.clone());
        self.models = models;
        self.config = config;
        let visible = self.visible();
        let selected = chosen.and_then(|c| visible.iter().position(|m| m.model == c));
        self.selected = selected.unwrap_or(0);
    }

    pub fn has_models(&self) -> bool {
        !self.models.is_empty()
    }

    /// The models the filter lets through, in config order. It looks at the
    /// provider, the model's name and its id, in any case.
    pub fn visible(&self) -> Vec<&ModelInfo> {
        let filter = self.filter.text().to_lowercase();
        self.models
            .iter()
            .filter(|m| {
                // `model` is `provider/name`, so it holds both.
                m.model.to_lowercase().contains(&filter) || m.id.to_lowercase().contains(&filter)
            })
            .collect()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The selected model; `None` if the filter matches nothing.
    pub fn chosen(&self) -> Option<&ModelInfo> {
        self.visible().get(self.selected).copied()
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let last = self.visible().len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    /// A key that edits the filter; the selection goes to the first match.
    pub fn edit(&mut self, edit: Edit) {
        if edit != Edit::Newline {
            self.filter.edit(edit);
            self.selected = 0;
        }
    }

    /// Pasted text goes to the filter, as one line.
    pub fn paste(&mut self, text: &str) {
        self.filter.paste(&text.replace(['\r', '\n'], " "));
        self.selected = 0;
    }
}
