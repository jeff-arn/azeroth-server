//! `ChatChannels.dbc` products: built-in channel lookup
//! by client-sent name (`GetChannelEntryFor`,
//! `DBCStores.cpp:523-549`). Built by the mangosd binary
//! from generic DBC rows, like
//! [`crate::player_info::RaceClassDbc`].

/// `Channel::ChannelDBCFlags` bits (`Channel.h:105-117`)
/// consumed by the channel constructor.
pub mod dbc_flags {
    pub const TRADE: u32 = 0x00008;
    pub const CITY_ONLY2: u32 = 0x00020;
    pub const LFG: u32 = 0x40000;
}

/// One `ChatChannelsEntry` subset (`DBCStructure.h:76-85`).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ChatChannelEntry {
    pub channel_id: u32,
    pub flags: u32,
    /// Locale name patterns, e.g. `General - %s`.
    pub patterns: Vec<String>,
}

/// Built-in channel table in DBC file order (lookups scan
/// from the first row, like `sChatChannelsStore`).
#[derive(Clone, Debug, Default)]
pub struct ChatChannelDbc {
    pub entries: Vec<ChatChannelEntry>,
}

impl ChatChannelDbc {
    /// `GetChannelEntryFor(name)`: strip `%s` from each
    /// locale pattern and take the first entry whose
    /// pattern is a substring of the client-sent name.
    #[must_use]
    pub fn entry_for_name(&self, name: &str) -> Option<&ChatChannelEntry> {
        self.entries.iter().find(|entry| {
            entry.patterns.iter().any(|pattern| {
                if pattern.is_empty() {
                    return false;
                }
                let stripped = pattern.replacen("%s", "", 1);
                name.contains(&stripped)
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dbc() -> ChatChannelDbc {
        ChatChannelDbc {
            entries: vec![
                ChatChannelEntry {
                    channel_id: 1,
                    flags: 0x3,
                    patterns: vec!["General - %s".into()],
                },
                ChatChannelEntry {
                    channel_id: 2,
                    flags: 0x3B,
                    patterns: vec!["Trade - %s".into()],
                },
                ChatChannelEntry {
                    channel_id: 23,
                    flags: 0x10004,
                    patterns: vec!["WorldDefense".into()],
                },
            ],
        }
    }

    #[test]
    fn zone_suffixed_names_match_patterns() {
        let dbc = dbc();
        assert_eq!(
            dbc.entry_for_name("General - Durotar")
                .map(|e| e.channel_id),
            Some(1)
        );
        assert_eq!(
            dbc.entry_for_name("Trade - City").map(|e| e.channel_id),
            Some(2)
        );
        assert_eq!(
            dbc.entry_for_name("WorldDefense").map(|e| e.channel_id),
            Some(23)
        );
        assert!(dbc.entry_for_name("MyCustom").is_none());
    }
}
