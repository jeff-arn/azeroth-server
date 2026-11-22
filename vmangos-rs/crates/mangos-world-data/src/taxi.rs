//! Taxi flight data (`TaxiNodes.dbc`, `TaxiPath.dbc`,
//! `TaxiPathNode.dbc`, `taxi_path_transitions`):
//! the fork's `sTaxiPathSetBySource`,
//! `sTaxiPathNodesByPath`, and `sTaxiNodesMask`
//! products plus `ObjectMgr` taxi lookups
//! (`GetNearestTaxiNode`, `GetTaxiPath`,
//! `GetTaxiMountDisplayId` data half) and
//! `ObjectMgr::LoadTaxiPathTransitions`
//! (`ObjectMgr.cpp:6981-7085`, `8975-9040`;
//! `DBCStores.cpp:340-405`).
//!
//! The global nodes mask includes every existing
//! node that originates at least one non-spell
//! (non-`SPELL_EFFECT_SEND_TAXI`) path, exactly as
//! the fork builds it; nodes with no outgoing paths
//! at all are also included (fork quirk: the
//! spell-path exclusion only applies when a source
//! set exists and is non-empty).

use std::collections::{HashMap, HashSet};

use mysql::prelude::Queryable;

use mangos_db::DbError;

/// `TaxiMaskSize` (`DBCStructure.h:833`).
pub const TAXI_MASK_SIZE: usize = 8;
/// `SPELL_EFFECT_SEND_TAXI` (`SpellDefines.h`).
const EFFECT_SEND_TAXI: u32 = 123;

/// One `TaxiNodesEntry` (name locales dropped).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiNode {
    pub id: u32,
    pub map_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// `[horde, alliance]` mount creature entries.
    pub mount_creature_id: [u32; 2],
}

/// One `TaxiPathEntry`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaxiPath {
    pub id: u32,
    pub from: u32,
    pub to: u32,
    pub price: u32,
}

/// One `TaxiPathNodeEntry`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaxiPathNode {
    pub path: u32,
    pub index: u32,
    pub map_id: u32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub action_flag: u32,
    pub delay: u32,
}

/// One `taxi_path_transitions` row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaxiPathTransition {
    pub in_path: u32,
    pub out_path: u32,
    pub in_node: u32,
    pub out_node: u32,
}

/// Loaded taxi graph.
#[derive(Clone, Debug, Default)]
pub struct TaxiStore {
    nodes: HashMap<u32, TaxiNode>,
    max_node_id: u32,
    /// source node -> destination node -> path.
    paths_by_source: HashMap<u32, HashMap<u32, TaxiPath>>,
    paths_by_id: HashMap<u32, TaxiPath>,
    /// path id -> nodes in index order.
    path_nodes: HashMap<u32, Vec<TaxiPathNode>>,
    /// `sTaxiNodesMask`.
    pub nodes_mask: [u32; TAXI_MASK_SIZE],
    transitions: Vec<TaxiPathTransition>,
}

impl TaxiStore {
    /// Assembles the store from DBC row sets. `spell_paths`
    /// is the set of taxi path ids referenced by
    /// `SPELL_EFFECT_SEND_TAXI` spell effects (fork
    /// nodes-mask exclusion input).
    #[must_use]
    pub fn from_rows(
        nodes: Vec<TaxiNode>,
        paths: Vec<TaxiPath>,
        mut path_node_rows: Vec<TaxiPathNode>,
        spell_paths: &HashSet<u32>,
        transitions: Vec<TaxiPathTransition>,
    ) -> Self {
        let max_node_id = nodes.iter().map(|n| n.id).max().map_or(0, |max| max + 1);
        let nodes: HashMap<u32, TaxiNode> = nodes.into_iter().map(|n| (n.id, n)).collect();
        let mut paths_by_source: HashMap<u32, HashMap<u32, TaxiPath>> = HashMap::new();
        let mut paths_by_id = HashMap::new();
        for path in paths {
            paths_by_id.insert(path.id, path);
            paths_by_source
                .entry(path.from)
                .or_default()
                .insert(path.to, path);
        }
        path_node_rows.sort_unstable_by_key(|n| (n.path, n.index));
        let mut path_nodes: HashMap<u32, Vec<TaxiPathNode>> = HashMap::new();
        for node in path_node_rows {
            path_nodes.entry(node.path).or_default().push(node);
        }
        // `LoadTaxiPathTransitions` row gates: both
        // paths exist and node indexes are in range.
        // Duplicate (in, out) pairs are kept like the
        // fork (its duplicate `continue` escapes only
        // the inner scan loop); lookups take the
        // first match either way.
        let transitions: Vec<TaxiPathTransition> = transitions
            .into_iter()
            .filter(|t| {
                paths_by_id.contains_key(&t.in_path)
                    && paths_by_id.contains_key(&t.out_path)
                    && (t.in_node as usize) < path_nodes.get(&t.in_path).map_or(0, Vec::len)
                    && (t.out_node as usize) < path_nodes.get(&t.out_path).map_or(0, Vec::len)
            })
            .collect();
        let mut nodes_mask = [0u32; TAXI_MASK_SIZE];
        for i in 1..max_node_id {
            if !nodes.contains_key(&i) {
                continue;
            }
            if let Some(outgoing) = paths_by_source.get(&i)
                && !outgoing.is_empty()
                && outgoing.values().all(|p| spell_paths.contains(&p.id))
            {
                continue;
            }
            let field = ((i - 1) / 32) as usize;
            if field < TAXI_MASK_SIZE {
                nodes_mask[field] |= 1 << ((i - 1) % 32);
            }
        }
        Self {
            nodes,
            max_node_id,
            paths_by_source,
            paths_by_id,
            path_nodes,
            nodes_mask,
            transitions,
        }
    }

    /// Builds a store from parts (test fixtures; the
    /// nodes mask covers every listed node).
    #[must_use]
    pub fn from_parts_for_tests(
        nodes: Vec<TaxiNode>,
        paths: Vec<TaxiPath>,
        path_node_rows: Vec<TaxiPathNode>,
    ) -> Self {
        Self::from_rows(nodes, paths, path_node_rows, &HashSet::new(), Vec::new())
    }

    /// `GetTaxiNodeEntry`.
    #[must_use]
    pub fn node(&self, id: u32) -> Option<&TaxiNode> {
        self.nodes.get(&id)
    }

    /// `sTaxiPathStore.LookupEntry`.
    #[must_use]
    pub fn path_by_id(&self, id: u32) -> Option<&TaxiPath> {
        self.paths_by_id.get(&id)
    }

    /// `GetTaxiPath(source, destination)`: `(path, cost)`,
    /// `(0, 0)` when no direct path exists.
    #[must_use]
    pub fn taxi_path(&self, source: u32, destination: u32) -> (u32, u32) {
        self.paths_by_source
            .get(&source)
            .and_then(|set| set.get(&destination))
            .map_or((0, 0), |path| (path.id, path.price))
    }

    /// `sTaxiPathNodesByPath[path]`.
    #[must_use]
    pub fn path_nodes(&self, path: u32) -> &[TaxiPathNode] {
        self.path_nodes.get(&path).map_or(&[], Vec::as_slice)
    }

    /// Whether any path data exists for `path`
    /// (transition-loader validity gate).
    #[must_use]
    pub fn has_path(&self, path: u32) -> bool {
        self.paths_by_id.contains_key(&path)
    }

    /// First transition from `in_path` to `out_path`
    /// (`GetTaxiPathTransitionsMapBounds` scan).
    #[must_use]
    pub fn transition(&self, in_path: u32, out_path: u32) -> Option<&TaxiPathTransition> {
        self.transitions
            .iter()
            .find(|t| t.in_path == in_path && t.out_path == out_path)
    }

    /// `GetNearestTaxiNode`: nearest network node on
    /// `map_id` whose mount exists for the team
    /// (`alliance` selects the mount slot).
    #[must_use]
    pub fn nearest_taxi_node(&self, x: f32, y: f32, z: f32, map_id: u32, alliance: bool) -> u32 {
        let mount_slot = usize::from(alliance);
        let mut found = false;
        let mut best_dist = 0.0f32;
        let mut best_id = 0u32;
        for i in 1..self.max_node_id {
            let Some(node) = self.nodes.get(&i) else {
                continue;
            };
            if node.map_id != map_id || node.mount_creature_id[mount_slot] == 0 {
                continue;
            }
            let field = ((i - 1) / 32) as usize;
            if field >= TAXI_MASK_SIZE || self.nodes_mask[field] & (1 << ((i - 1) % 32)) == 0 {
                continue;
            }
            let dist2 = (node.x - x).powi(2) + (node.y - y).powi(2) + (node.z - z).powi(2);
            if !found || dist2 < best_dist {
                found = true;
                best_dist = dist2;
                best_id = i;
            }
        }
        best_id
    }

    /// Mount creature entry for a node/team
    /// (`GetTaxiMountDisplayId` creature half).
    #[must_use]
    pub fn mount_creature_id(&self, node_id: u32, alliance: bool, allowed_alt_team: bool) -> u32 {
        let Some(node) = self.nodes.get(&node_id) else {
            return 0;
        };
        let (primary, alt) = if alliance { (1, 0) } else { (0, 1) };
        let creature = node.mount_creature_id[primary];
        if creature == 0 && allowed_alt_team {
            node.mount_creature_id[alt]
        } else {
            creature
        }
    }

    /// Loaded node/path/vertex counts for boot logs.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize) {
        (
            self.nodes.len(),
            self.paths_by_id.len(),
            self.path_nodes.values().map(Vec::len).sum(),
        )
    }
}

/// Collects `SPELL_EFFECT_SEND_TAXI` path ids from a
/// spell set (`misc_value` of each send-taxi effect).
#[must_use]
pub fn spell_taxi_paths<'a>(
    spells: impl Iterator<Item = &'a mangos_spells::SpellEntry>,
) -> HashSet<u32> {
    let mut out = HashSet::new();
    for spell in spells {
        for effect in &spell.effects {
            if effect.effect == EFFECT_SEND_TAXI
                && let Ok(path) = u32::try_from(effect.misc_value)
            {
                out.insert(path);
            }
        }
    }
    out
}

/// `ObjectMgr::LoadTaxiPathTransitions` (rows whose
/// in/out paths do not exist are dropped by the
/// caller-side gate against the assembled store).
///
/// # Errors
///
/// Returns the query error.
pub fn load_taxi_path_transitions(
    conn: &mut mysql::Conn,
    client_build: u32,
) -> Result<Vec<TaxiPathTransition>, DbError> {
    let rows: Vec<(u32, u32, u32, u32)> = conn.query_map(
        format!(
            "SELECT in_path, out_path, in_node, out_node \
             FROM taxi_path_transitions WHERE build_min <= {client_build}"
        ),
        |(in_path, out_path, in_node, out_node)| (in_path, out_path, in_node, out_node),
    )?;
    Ok(rows
        .into_iter()
        .map(
            |(in_path, out_path, in_node, out_node)| TaxiPathTransition {
                in_path,
                out_path,
                in_node,
                out_node,
            },
        )
        .collect())
}
