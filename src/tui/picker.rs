//! The state of the model picker (see `spec/tui.md`, "Model picker"): the
//! models from the server, the filter and the selected row. `App` routes keys
//! here; `view::picker` draws it.

use std::path::PathBuf;

use crate::protocol::{DefaultFrom, ModelInfo};

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
    /// What sets the default the `default` mark shows (RFC 0021).
    pub default_from: DefaultFrom,
    /// "Also make it the default for new sessions", ticked with `Space`.
    pub make_default: bool,
}

impl Picker {
    /// Starts on the current model; for a new session on `default_model`.
    pub fn new(
        models: Vec<ModelInfo>,
        config: PathBuf,
        default_from: DefaultFrom,
        current: &str,
        purpose: Purpose,
    ) -> Picker {
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
            default_from,
            make_default: false,
        }
    }

    /// A newer list, after a reload: the same model stays selected if it can.
    pub fn set_models(&mut self, models: Vec<ModelInfo>, config: PathBuf, from: DefaultFrom) {
        let chosen = self.chosen().map(|m| m.model.clone());
        self.models = models;
        self.config = config;
        self.default_from = from;
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

    /// The default new sessions get now, as the `default` mark shows it.
    pub fn default_model(&self) -> Option<&ModelInfo> {
        self.models.iter().find(|m| m.default)
    }

    /// The selected model is the user file's default already: there is
    /// nothing to save.
    pub fn already_default(&self) -> bool {
        self.default_from == DefaultFrom::User && self.chosen().is_some_and(|m| m.default)
    }

    /// `Space`: tick or untick "Also make it the default".
    pub fn toggle_default(&mut self) {
        if !self.already_default() {
            self.make_default = !self.make_default;
        }
    }

    /// The model to save as `default_model` when `Enter` takes the selected
    /// one: only with the tick, and only if it is not the default already.
    pub fn default_to_save(&self) -> Option<String> {
        if !self.make_default || self.already_default() {
            return None;
        }
        self.chosen().map(|m| m.model.clone())
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
