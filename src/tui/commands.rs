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
    later(command("sessions", "Open the session list")),
    later(optional(
        command("new", "Start a new session and attach"),
        "[NAME]",
    )),
    later(required(
        command("attach", "Attach to another session"),
        "NAME",
    )),
    required(command("rename", "Rename the session"), "NAME"),
    command("kill", "End the session, after a question"),
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
        assert_eq!(names("m"), ["model", "permissions", "rename", "commit"]);
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
