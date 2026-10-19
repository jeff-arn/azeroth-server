//! GM command surface contract (`Chat.cpp`):
//! the static command table (generated into
//! `commands_table.rs` from the fork's
//! `getCommandTable`), DB overrides from the world
//! `command` table, RBAC permission masks, the
//! `FindCommand` abbreviation search, availability,
//! and help rendering. Command *handlers* trail
//! (architecture §13.2 Phase 5: "port the
//! table/permissions contract; 23.6k LOC can
//! trail"): the actor registers real handlers by
//! full name (only `help`/`commands` today) and
//! every other known command takes the fork's
//! handler-returned-false path (help text or
//! `LANG_CMD_SYNTAX` plus the subcommand list).

use std::collections::HashMap;

use mangos_db::commands::{CommandOverrideRow, RbacRows};

#[path = "commands_table.rs"]
mod table;

/// `LANG_NO_HELP_CMD`.
pub const LANG_NO_HELP_CMD: i32 = 5;
/// `LANG_NO_CMD`.
pub const LANG_NO_CMD: i32 = 6;
/// `LANG_NO_SUBCMD`.
pub const LANG_NO_SUBCMD: i32 = 7;
/// `LANG_SUBCMDS_LIST`.
pub const LANG_SUBCMDS_LIST: i32 = 8;
/// `LANG_AVIABLE_CMD`.
pub const LANG_AVIABLE_CMD: i32 = 9;
/// `LANG_CMD_SYNTAX`.
pub const LANG_CMD_SYNTAX: i32 = 10;
/// `LANG_COMMAND_UNAVAILABLE`.
pub const LANG_COMMAND_UNAVAILABLE: i32 = 50;

/// `COMMAND_FLAGS_ONLY_ON_SELF` (DB `flags` bit 0;
/// see `ExecuteCommand`).
pub const FLAG_ONLY_ON_SELF: u8 = 0x01;
/// `COMMAND_FLAGS_CRITICAL` (DB `flags` bit 1).
pub const FLAG_CRITICAL: u8 = 0x02;

/// One static table row (the generated data shape).
pub struct CommandDef {
    pub name: &'static str,
    pub security: u8,
    pub allow_console: bool,
    pub has_handler: bool,
    pub help: &'static str,
    pub children: &'static [CommandDef],
}

/// One live command node (`ChatCommand` after DB
/// overrides and RBAC masks are applied).
#[derive(Clone, Debug)]
pub struct Command {
    pub name: &'static str,
    pub security: u8,
    pub allow_console: bool,
    pub has_handler: bool,
    pub help: String,
    pub children: Vec<Command>,
    pub flags: u8,
    pub full_name: String,
    pub permission_mask: u32,
}

impl Command {
    fn from_def(def: &CommandDef) -> Self {
        Self {
            name: def.name,
            security: def.security,
            allow_console: def.allow_console,
            has_handler: def.has_handler,
            help: def.help.to_owned(),
            children: def.children.iter().map(Self::from_def).collect(),
            flags: 0,
            full_name: String::new(),
            permission_mask: 0,
        }
    }
}

/// RBAC state (`LoadRbacPermissions` maps).
#[derive(Clone, Debug, Default)]
pub struct Rbac {
    pub permission_names: HashMap<u32, String>,
    pub granted: HashMap<u32, u32>,
    pub banned: HashMap<u32, u32>,
}

/// `enum ChatCommandSearchResult`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchResult {
    Ok,
    Unknown,
    UnknownSubcommand,
}

/// Outcome of a `FindCommand` walk: `path` indexes
/// from the root to the found command (Ok) or the
/// last matched parent (`UnknownSubcommand`); `tail`
/// is the unparsed remainder; `last_name` is the
/// final parsed token as typed.
#[derive(Clone, Debug)]
pub struct FindOutcome {
    pub result: SearchResult,
    pub path: Vec<usize>,
    pub tail: String,
    pub last_name: String,
}

/// `ChatHandler::hasStringAbbr`: case-insensitive
/// prefix match; the empty command name matches
/// everything, an empty abbreviation matches no
/// non-empty name.
#[must_use]
pub fn has_string_abbr(name: &str, part: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    if part.is_empty() {
        return false;
    }
    name.len() >= part.len() && name[..part.len()].eq_ignore_ascii_case(part)
}

/// The live command table plus RBAC state.
#[derive(Clone, Debug)]
pub struct CommandTable {
    roots: Vec<Command>,
    pub rbac: Rbac,
}

impl Default for CommandTable {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandTable {
    /// Builds the table from the static defs and
    /// fills full names (`FillFullCommandsName`).
    #[must_use]
    pub fn new() -> Self {
        let mut roots: Vec<Command> = table::ROOT_COMMAND_TABLE
            .iter()
            .map(Command::from_def)
            .collect();
        fill_full_names(&mut roots, "");
        Self {
            roots,
            rbac: Rbac::default(),
        }
    }

    /// Total command count across all tables.
    #[must_use]
    pub fn count(&self) -> usize {
        fn count(nodes: &[Command]) -> usize {
            nodes.iter().map(|node| 1 + count(&node.children)).sum()
        }
        count(&self.roots)
    }

    /// Root commands (top-level list for `.help`).
    #[must_use]
    pub fn roots(&self) -> &[Command] {
        &self.roots
    }

    /// Resolves a path from [`FindOutcome`].
    #[must_use]
    pub fn get(&self, path: &[usize]) -> Option<&Command> {
        let mut nodes = &self.roots;
        let mut current = None;
        for &index in path {
            current = nodes.get(index);
            nodes = &current?.children;
        }
        current
    }

    fn get_mut(&mut self, path: &[usize]) -> Option<&mut Command> {
        fn walk<'a>(nodes: &'a mut [Command], path: &[usize]) -> Option<&'a mut Command> {
            let (first, rest) = path.split_first()?;
            let node = nodes.get_mut(*first)?;
            if rest.is_empty() {
                Some(node)
            } else {
                walk(&mut node.children, rest)
            }
        }
        walk(&mut self.roots, path)
    }

    /// `FindCommand` over the root table.
    #[must_use]
    pub fn find(&self, text: &str, exactly_name: bool) -> FindOutcome {
        find_in(&self.roots, text, exactly_name, Vec::new())
    }

    /// `isAvailable`: security level first, RBAC ban
    /// masks deny, RBAC grant masks allow below the
    /// level.
    #[must_use]
    pub fn is_available(&self, cmd: &Command, security: u8, account_id: u32) -> bool {
        if security >= cmd.security {
            if let Some(mask) = self.rbac.banned.get(&account_id)
                && cmd.permission_mask & mask != 0
            {
                return false;
            }
            true
        } else {
            self.rbac
                .granted
                .get(&account_id)
                .is_some_and(|mask| cmd.permission_mask & mask != 0)
        }
    }

    /// Applies one `command` DB override
    /// (`SetDataForCommandInTable`); returns the
    /// fork's log lines for the caller to emit.
    pub fn apply_override(&mut self, row: &CommandOverrideRow) -> Vec<String> {
        let mut logs = Vec::new();
        let outcome = self.find(&row.name, true);
        match outcome.result {
            SearchResult::Ok => {
                if let Some(cmd) = self.get_mut(&outcome.path) {
                    if cmd.security != row.security {
                        logs.push(format!(
                            "Table `command` overwrite for command '{}' default security ({}) by {}",
                            row.name, cmd.security, row.security
                        ));
                    }
                    cmd.security = row.security;
                    cmd.help.clone_from(&row.help);
                    cmd.flags = row.flags;
                }
            }
            SearchResult::UnknownSubcommand => {
                if outcome.last_name.is_empty() {
                    logs.push(format!(
                        "Table `command` have command '{}' that only used with some subcommand selection, it can't have help or overwritten access level, skip.",
                        outcome.last_name
                    ));
                } else {
                    logs.push(format!(
                        "Table `command` have unexpected subcommand '{}' in command '{}', skip.",
                        outcome.last_name, row.name
                    ));
                }
            }
            SearchResult::Unknown => {
                logs.push(format!(
                    "Table `command` have nonexistent command '{}', skip.",
                    outcome.last_name
                ));
            }
        }
        logs
    }

    /// Applies the RBAC rows (`LoadRbacPermissions`);
    /// returns the fork's log lines.
    pub fn apply_rbac(&mut self, rows: &RbacRows) -> Vec<String> {
        let mut logs = Vec::new();
        for (id, name) in &rows.permissions {
            if *id >= 32 {
                logs.push(
                    "RBAC permission ids are stored as an uint32 mask and cannot exceed id 31!"
                        .to_owned(),
                );
                continue;
            }
            if name.is_empty() {
                logs.push("RBAC permission names should not be empty!".to_owned());
                continue;
            }
            self.rbac.permission_names.insert(*id, name.clone());
        }
        for (account_id, permission_id, granted) in &rows.account_permissions {
            if !self.rbac.permission_names.contains_key(permission_id) {
                logs.push(format!(
                    "Unknown RBAC permission id {permission_id} assigned to account {account_id}!"
                ));
                continue;
            }
            let map = if *granted {
                &mut self.rbac.granted
            } else {
                &mut self.rbac.banned
            };
            *map.entry(*account_id).or_default() |= 1u32 << permission_id;
        }
        for (command, permission_id) in &rows.command_permissions {
            if !self.rbac.permission_names.contains_key(permission_id) {
                logs.push(format!(
                    "Unknown RBAC permission id {permission_id} assigned to command '{command}'!"
                ));
                continue;
            }
            let outcome = self.find(command, true);
            match outcome.result {
                SearchResult::Ok => {
                    if let Some(cmd) = self.get_mut(&outcome.path) {
                        cmd.permission_mask |= 1u32 << permission_id;
                    }
                }
                SearchResult::UnknownSubcommand => {
                    if outcome.last_name.is_empty() {
                        logs.push(format!(
                            "Table `command` have command '{}' that only used with some subcommand selection, it can't have help or overwritten access level, skip.",
                            outcome.last_name
                        ));
                    } else {
                        logs.push(format!(
                            "Table `command` have unexpected subcommand '{}' in command '{command}', skip.",
                            outcome.last_name
                        ));
                    }
                }
                SearchResult::Unknown => {
                    logs.push(format!(
                        "Table `command` have nonexistent command '{}', skip.",
                        outcome.last_name
                    ));
                }
            }
        }
        logs
    }

    /// `ShowHelpForSubCommands` list body (without
    /// the header line), or `None` when nothing is
    /// available to the viewer.
    #[must_use]
    pub fn subcommand_list(
        &self,
        nodes: &[Command],
        security: u8,
        account_id: u32,
    ) -> Option<String> {
        let mut list = String::new();
        for node in nodes {
            if !self.is_available(node, security, account_id) {
                continue;
            }
            list += "\n    ";
            list += node.name;
            if !node.children.is_empty() {
                list += " ...";
            }
        }
        if list.is_empty() { None } else { Some(list) }
    }
}

fn fill_full_names(nodes: &mut [Command], prefix: &str) {
    for node in nodes {
        node.full_name = if prefix.is_empty() {
            node.name.to_owned()
        } else if node.name.is_empty() {
            prefix.to_owned()
        } else {
            format!("{prefix} {}", node.name)
        };
        let child_prefix = node.full_name.clone();
        fill_full_names(&mut node.children, &child_prefix);
    }
}

/// Splits the first space-delimited token exactly
/// like `FindCommand` (token runs to the first
/// space, then spaces are skipped).
fn split_token(text: &str) -> (&str, &str) {
    let end = text.find(' ').unwrap_or(text.len());
    let (token, rest) = text.split_at(end);
    (token, rest.trim_start_matches(' '))
}

fn find_in(nodes: &[Command], text: &str, exactly_name: bool, prefix: Vec<usize>) -> FindOutcome {
    let (cmd, rest) = split_token(text);
    for (index, node) in nodes.iter().enumerate() {
        let matched = if exactly_name {
            node.name == cmd
        } else {
            has_string_abbr(node.name, cmd)
        };
        if !matched {
            continue;
        }
        if !node.children.is_empty() {
            let mut child_prefix = prefix.clone();
            child_prefix.push(index);
            let sub = find_in(&node.children, rest, exactly_name, child_prefix);
            match sub.result {
                SearchResult::Ok => {
                    // "" direct child: the tail restores
                    // to just after this command's name.
                    let direct = sub.path.len() == prefix.len() + 2;
                    if direct
                        && node
                            .children
                            .get(sub.path[prefix.len() + 1])
                            .is_some_and(|child| child.name.is_empty())
                    {
                        return FindOutcome {
                            tail: rest.to_owned(),
                            ..sub
                        };
                    }
                    return sub;
                }
                SearchResult::Unknown => {
                    let mut path = prefix;
                    path.push(index);
                    return FindOutcome {
                        result: SearchResult::UnknownSubcommand,
                        path,
                        tail: rest.to_owned(),
                        last_name: sub.last_name,
                    };
                }
                SearchResult::UnknownSubcommand => return sub,
            }
        }
        if !node.has_handler {
            continue;
        }
        let mut path = prefix;
        path.push(index);
        return FindOutcome {
            result: SearchResult::Ok,
            path,
            tail: rest.to_owned(),
            last_name: cmd.to_owned(),
        };
    }
    FindOutcome {
        result: SearchResult::Unknown,
        path: Vec::new(),
        tail: text.to_owned(),
        last_name: cmd.to_owned(),
    }
}

/// The text of `.help`/syntax outputs for one
/// resolved command lookup (`ShowHelpForCommand`
/// sequencing): returns (lines, found-anything).
/// `header_for` renders the available/subcommands
/// header lines via lang ids, so this stays a pure
/// string builder for tests.
pub struct HelpOutput {
    pub lines: Vec<HelpLine>,
    pub found: bool,
}

/// One help output unit: a lang string id (with an
/// optional `%s` argument) or a literal block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HelpLine {
    Lang(i32),
    LangArg(i32, String),
    Text(String),
}

impl CommandTable {
    /// `ShowHelpForCommand` over the root table.
    #[must_use]
    pub fn help_for(&self, text: &str, security: u8, account_id: u32) -> HelpOutput {
        self.help_for_in(&self.roots, text, true, security, account_id)
    }

    /// `ShowHelpForCommand` over one (sub)table; the
    /// unknown-command arm lists the table unless it
    /// is the root with a non-empty query.
    #[must_use]
    pub fn help_for_in(
        &self,
        nodes: &[Command],
        text: &str,
        is_root: bool,
        security: u8,
        account_id: u32,
    ) -> HelpOutput {
        fn get_in<'a>(nodes: &'a [Command], path: &[usize]) -> Option<&'a Command> {
            let mut current = None;
            let mut level = nodes;
            for &index in path {
                current = level.get(index);
                level = &current?.children;
            }
            current
        }
        let outcome = find_in(nodes, text, false, Vec::new());
        let mut lines = Vec::new();
        let (command, show_command, child_list) = match outcome.result {
            SearchResult::Ok => {
                let cmd = get_in(nodes, &outcome.path);
                let show = match cmd {
                    Some(found) if found.name.is_empty() && outcome.path.len() > 1 => {
                        get_in(nodes, &outcome.path[..outcome.path.len() - 1])
                    }
                    other => other,
                };
                (cmd, show, show.map(|node| node.children.as_slice()))
            }
            SearchResult::UnknownSubcommand => {
                let cmd = get_in(nodes, &outcome.path);
                (cmd, cmd, cmd.map(|node| node.children.as_slice()))
            }
            SearchResult::Unknown => {
                let list = if !is_root || text.is_empty() {
                    Some(nodes)
                } else {
                    None
                };
                (None, None, list)
            }
        };
        if let Some(cmd) = command
            && !cmd.help.is_empty()
        {
            lines.push(HelpLine::Text(cmd.help.clone()));
        }
        if let Some(children) = child_list
            && !children.is_empty()
            && let Some(list) = self.subcommand_list(children, security, account_id)
        {
            let list_is_root = is_root && std::ptr::eq(children, self.roots.as_slice());
            if list_is_root {
                lines.push(HelpLine::Lang(LANG_AVIABLE_CMD));
            } else {
                lines.push(HelpLine::LangArg(
                    LANG_SUBCMDS_LIST,
                    show_command.map_or_else(String::new, |node| node.name.to_owned()),
                ));
            }
            lines.push(HelpLine::Text(list));
            return HelpOutput { lines, found: true };
        }
        if let Some(cmd) = command
            && cmd.help.is_empty()
        {
            lines.push(HelpLine::Lang(LANG_NO_HELP_CMD));
        }
        HelpOutput {
            lines,
            found: command.is_some() || child_list.is_some(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_shape() {
        let table = CommandTable::new();
        assert_eq!(table.count(), 842);
        let partybot = table
            .roots()
            .iter()
            .find(|cmd| cmd.name == "partybot")
            .expect("partybot root");
        assert_eq!(partybot.security, 6);
        assert!(!partybot.children.is_empty());
        assert_eq!(partybot.full_name, "partybot");
        let debug = partybot
            .children
            .iter()
            .find(|cmd| cmd.name == "debug")
            .expect("partybot debug");
        assert_eq!(debug.full_name, "partybot debug");
        assert!(debug.has_handler);
    }

    #[test]
    fn abbr_and_find() {
        assert!(has_string_abbr("teleport", "tele"));
        assert!(has_string_abbr("teleport", "TELEPORT"));
        assert!(!has_string_abbr("teleport", "teleports"));
        assert!(!has_string_abbr("teleport", ""));
        assert!(has_string_abbr("", "anything"));
        let table = CommandTable::new();
        let found = table.find("partybot deb", false);
        assert_eq!(found.result, SearchResult::Ok);
        assert_eq!(
            table.get(&found.path).map(|c| c.full_name.as_str()),
            Some("partybot debug")
        );
        let unknown_sub = table.find("partybot nosuch", false);
        assert_eq!(unknown_sub.result, SearchResult::UnknownSubcommand);
        let unknown = table.find("nosuchcmd", false);
        assert_eq!(unknown.result, SearchResult::Unknown);
    }

    #[test]
    fn availability_and_rbac() {
        let mut table = CommandTable::new();
        let root_help = table
            .roots()
            .iter()
            .find(|cmd| cmd.name == "help")
            .expect("help root")
            .clone();
        assert!(table.is_available(&root_help, 0, 42));
        let rows = RbacRows {
            permissions: vec![(1, "test".into())],
            account_permissions: vec![(42, 1, false), (43, 1, true)],
            command_permissions: vec![("help".into(), 1)],
        };
        table.apply_rbac(&rows);
        let help = table
            .roots()
            .iter()
            .find(|cmd| cmd.name == "help")
            .expect("help root")
            .clone();
        assert!(!table.is_available(&help, 0, 42));
        assert!(table.is_available(&help, 0, 43));
    }

    #[test]
    fn override_changes_security() {
        let mut table = CommandTable::new();
        table.apply_override(&CommandOverrideRow {
            name: "help".into(),
            security: 3,
            help: "Shows help".into(),
            flags: 0,
        });
        let help = table.roots().iter().find(|cmd| cmd.name == "help").unwrap();
        assert_eq!((help.security, help.help.as_str()), (3, "Shows help"));
    }
}
