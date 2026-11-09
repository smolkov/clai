//! The `/` commands of the TUI and the search of the command list (see
//! `spec/tui.md`, "Command list"). `App::command` runs them.

/// A row of the command list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    /// Another name for the same command.
    pub alias: Option<&'static str>,
    /// The arguments as shown: `NAME`, `[NAME]`.
    pub arguments: Option<&'static str>,
    /// `Enter` completes it rather than running it without an argument.
    pub needs_argument: bool,
    pub description: &'static str,
    /// `false`: it comes with a later version.
    pub built: bool,
}

/// A built command without arguments; the functions below change it.
const fn command(name: &'static str, description: &'static str) -> Command {
    Command {
        name,
        alias: None,
        arguments: None,
        needs_argument: false,
        description,
        built: true,
    }
}

const fn later(command: Command) -> Command {
    Command {
        built: false,
        ..command
    }
}

const fn optional(command: Command, arguments: &'static str) -> Command {
    Command {
        arguments: Some(arguments),
        ..command
    }
}

const fn required(command: Command, arguments: &'static str) -> Command {
    Command {
        arguments: Some(arguments),
        needs_argument: true,
        ..command
    }
}

/// All commands, in the order of the table in the spec.
pub const COMMANDS: &[Command] = &[
    command("help", "Show keys and commands"),
    later(command("clear", "Start a new conversation in this session")),
    optional(
        command("model", "Change the model; without one, choose it"),
        "[PROVIDER/MODEL]",
    ),
    later(command(
        "permissions",
        "Show the permission rules in effect",
    )),
    command("sessions", "Open the session list"),
    optional(
        command("new", "Start a new session, choose its model"),
        "[NAME]",
    ),
    optional(
        command(
            "attach",
            "Attach to another session; without one, choose it",
        ),
        "[NAME]",
    ),
    required(command("rename", "Rename the session"), "NAME"),
    optional(
        command("status", "Show the status and statistics of a session"),
        "[NAME]",
    ),
    command("settings", "Change the TUI settings and save them"),
    optional(
        command("theme", "Choose the colour theme, saved as the default"),
        "[NAME]",
    ),
    Command {
        alias: Some("delete"),
        ..command("kill", "Delete the session, after a question")
    },
    later(optional(
        command("commit", "Commit the changes, after a question"),
        "[HINT]",
    )),
    Command {
        alias: Some("quit"),
        ..command("detach", "Leave; the session keeps running")
    },
];

impl Command {
    /// As the list shows it: `/model [PROVIDER/MODEL]`, `/detach, /quit`.
    pub fn shown(&self) -> String {
        let mut shown = format!("/{}", self.name);
        if let Some(alias) = self.alias {
            shown.push_str(&format!(", /{alias}"));
        }
        if let Some(arguments) = self.arguments {
            shown.push(' ');
            shown.push_str(arguments);
        }
        shown
    }

    /// The text `Tab` puts in the input.
    pub fn completed(&self) -> String {
        let space = if self.arguments.is_some() { " " } else { "" };
        format!("/{}{space}", self.name)
    }

    fn names(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name).chain(self.alias)
    }
}

/// What the list searches for if the input is `text`; `None` if the input is
/// not a search: no `/` first, a message written `//…`, or arguments follow.
pub fn search_text(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('/')?;
    (!rest.starts_with('/') && !rest.contains(char::is_whitespace)).then_some(rest)
}

/// The commands whose argument the list completes.
pub const COMPLETED: &[&str] = &["attach", "model", "status", "theme"];

/// A value the argument list offers: what goes in the input, and a hint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Value {
    pub value: String,
    pub description: String,
    /// The description is searched too: a model's id, not a session's
    /// workspace, which many sessions share.
    pub search_description: bool,
}

/// The command and the typed part of its argument, if the input is one of
/// [`COMPLETED`] followed by a space and at most one word.
pub fn argument_text(text: &str) -> Option<(&'static str, &str)> {
    let rest = text.strip_prefix('/')?;
    let (name, argument) = rest.split_once(' ')?;
    let name = COMPLETED.iter().find(|c| **c == name)?;
    (!argument.contains(char::is_whitespace)).then_some((name, argument))
}

/// The values for a search, in any case: those that start with it first, then
/// those that contain it, each in the given order.
pub fn search_values(values: Vec<Value>, text: &str) -> Vec<Value> {
    let text = text.to_lowercase();
    let starts = |v: &Value| v.value.to_lowercase().starts_with(&text);
    let (mut first, rest): (Vec<Value>, Vec<Value>) = values.into_iter().partition(starts);
    first.extend(rest.into_iter().filter(|v| {
        v.value.to_lowercase().contains(&text)
            || (v.search_description && v.description.to_lowercase().contains(&text))
    }));
    first
}

/// The commands for a search, in any case: names that start with it first, then
/// those that contain it, each in table order.
pub fn search(text: &str) -> Vec<&'static Command> {
    let text = text.to_lowercase();
    let starting = |c: &&Command| c.names().any(|n| n.starts_with(&text));
    let containing = |c: &&Command| !starting(c) && c.names().any(|n| n.contains(&text));
    COMMANDS
        .iter()
        .filter(starting)
        .chain(COMMANDS.iter().filter(containing))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<&'static str> {
        search(text).iter().map(|c| c.name).collect()
    }

    #[test]
    fn an_empty_search_lists_all_in_table_order() {
        assert_eq!(
            names(""),
            COMMANDS.iter().map(|c| c.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn names_that_start_with_the_text_come_first() {
        // `model` starts with it; the others only contain it.
        assert_eq!(
            names("m"),
            ["model", "permissions", "rename", "theme", "commit"]
        );
        // Each group keeps the table order.
        assert_eq!(
            names("e"),
            [
                "help",
                "clear",
                "model",
                "permissions",
                "sessions",
                "new",
                "rename",
                "settings",
                "theme",
                "kill",
                "detach"
            ]
        );
    }

    #[test]
    fn the_case_does_not_matter() {
        assert_eq!(names("MOD"), ["model"]);
    }

    #[test]
    fn either_name_finds_a_row() {
        assert_eq!(names("qu"), ["detach"]);
        assert_eq!(names("det"), ["detach"]);
    }

    #[test]
    fn no_match() {
        assert!(names("zzz").is_empty());
    }

    #[test]
    fn what_is_a_search() {
        assert_eq!(search_text("/"), Some(""));
        assert_eq!(search_text("/mo"), Some("mo"));
        for text in ["", "mo", "//", "//mo", "/model a", "/model ", "/a\nb"] {
            assert_eq!(search_text(text), None, "{text}");
        }
    }

    #[test]
    fn what_is_an_argument_search() {
        assert_eq!(argument_text("/attach "), Some(("attach", "")));
        assert_eq!(argument_text("/model loc"), Some(("model", "loc")));
        for text in [
            "/attach",
            "/rename x",
            "/model a b",
            "/model a\nb",
            "attach x",
        ] {
            assert_eq!(argument_text(text), None, "{text}");
        }
    }

    #[test]
    fn values_that_start_with_the_text_come_first() {
        let value = |v: &str, d: &str| Value {
            value: v.into(),
            description: d.into(),
            search_description: d != "hidden",
        };
        let values = vec![
            value("anthropic/sonnet", "claude-sonnet"),
            value("local/qwen", "qwen3-coder"),
            value("anthropic/qwen-proxy", "hidden"),
        ];
        let found = |text| -> Vec<String> {
            search_values(values.clone(), text)
                .into_iter()
                .map(|v| v.value)
                .collect()
        };
        assert_eq!(found("LOC"), ["local/qwen"]);
        assert_eq!(found("qwen"), ["local/qwen", "anthropic/qwen-proxy"]);
        assert_eq!(found("claude"), ["anthropic/sonnet"]);
        assert!(found("hidden").is_empty());
        assert_eq!(found("").len(), 3);
        assert!(found("zzz").is_empty());
    }

    #[test]
    fn rows_as_shown_and_completed() {
        let find = |name| COMMANDS.iter().find(|c| c.name == name).unwrap();
        assert_eq!(find("help").shown(), "/help");
        assert_eq!(find("help").completed(), "/help");
        assert_eq!(find("detach").shown(), "/detach, /quit");
        assert_eq!(find("model").shown(), "/model [PROVIDER/MODEL]");
        assert_eq!(find("model").completed(), "/model ");
        assert!(find("rename").needs_argument);
        assert!(!find("model").needs_argument);
    }
}
