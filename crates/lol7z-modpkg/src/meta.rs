//! The metadata document, `_meta_/info.msgpack` (section 11 of the format), as serde
//! types: read to show what a package is and to make its `mod.config.json`, written
//! back from the config when packing.
//!
//! The schema is additive, so a key not modelled is kept as a [`Value`] in `extra` and
//! written back as it was; a key missing is its default. A layer's `game_data` is kept
//! as the JSON it is, since the project keeps it as `content/<layer>/game_data.json`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Longest document read at open, in bytes.
pub const MAX_INFO: u64 = 4 << 20;

/// The highest schema version this crate knows.
pub const SCHEMA_VERSION: u32 = 5;

/// What this crate writes as `generator`.
pub const GENERATOR: &str = concat!(env!("CARGO_PKG_NAME"), " ", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_version: Option<u32>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub distributor: Option<Distributor>,
    #[serde(default)]
    pub authors: Vec<Author>,
    #[serde(default)]
    pub license: License,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub champions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub maps: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashtables: Vec<Hashtable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    /// Keys this crate doesn't know, kept as they were.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Author {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
}

/// A `type` key selects the shape; a type not known here reads as `Unknown`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum License {
    #[default]
    None,
    Spdx {
        spdx_id: String,
    },
    Custom {
        name: String,
        #[serde(default)]
        url: String,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Distributor {
    #[serde(default)]
    pub site_id: String,
    #[serde(default)]
    pub site_name: String,
    #[serde(default)]
    pub site_url: String,
    #[serde(default)]
    pub mod_id: String,
}

/// Informational: the layer table is the source of which layers exist and their
/// priorities.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct LayerMetadata {
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Locale, then stringtable field, to the replacement string, in document order.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub string_overrides: IndexMap<String, IndexMap<String, String>>,
    /// The layer's game-data declaration document, schema 4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_data: Option<Value>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

/// One embedded hashtable, `_meta_/hashes/<file>`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Hashtable {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub algorithm: String,
    #[serde(default)]
    pub bits: u32,
}

impl Metadata {
    /// `None` for bytes that aren't a MessagePack map of these shapes.
    pub fn parse(bytes: &[u8]) -> Option<Metadata> {
        rmp_serde::from_slice(bytes).ok()
    }

    pub fn to_msgpack(&self) -> Vec<u8> {
        rmp_serde::to_vec_named(self).expect("a document of these types serializes")
    }

    /// `Display Name 1.0.0 by a, b`, leaving out what's empty; `None` when all is.
    pub fn summary(&self) -> Option<String> {
        let name = if self.display_name.is_empty() {
            &self.name
        } else {
            &self.display_name
        };
        let authors: Vec<&str> = self
            .authors
            .iter()
            .map(|a| a.name.as_str())
            .filter(|n| !n.is_empty())
            .collect();
        let mut parts: Vec<String> = Vec::new();
        for s in [name, &self.version] {
            if !s.is_empty() {
                parts.push(s.clone());
            }
        }
        if !authors.is_empty() {
            parts.push(format!("by {}", authors.join(", ")));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    pub fn layer(&self, name: &str) -> Option<&LayerMetadata> {
        self.layers.iter().find(|l| l.name == name)
    }
}
