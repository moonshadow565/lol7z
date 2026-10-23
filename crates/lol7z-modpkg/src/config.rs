//! `mod.config.json`: the mod project's config, shown for a package in place of its
//! metadata document and read back when packing. The shapes are the project's
//! (`ltk_mod_project`): authors as a string or a name-role object, a license as an SPDX
//! string or a name-url object, hashtable paths under `hashes/`.

use crate::meta::{
    Author, Distributor, GENERATOR, Hashtable, LayerMetadata, License, Metadata, SCHEMA_VERSION,
};
use crate::path::{self, BASE, HASHES, HASHES_PREFIX};
use crate::toc::Layer;
use indexmap::IndexMap;
use lol7z_api::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const FILE: &str = "mod.config.json";
/// Longest config read when packing, in bytes.
pub const MAX_CONFIG: u64 = 4 << 20;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub authors: Vec<ConfigAuthor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<ConfigLicense>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub champions: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub maps: Vec<String>,
    /// Absent means one layer, `base`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<ConfigLayer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashtables: Vec<Hashtable>,
    /// Keys this crate doesn't know, kept as they were: the project's own, and a
    /// package's `distributor`.
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ConfigAuthor {
    Name(String),
    Role { name: String, role: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum ConfigLicense {
    Spdx(String),
    Custom {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ConfigLayer {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub priority: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub string_overrides: IndexMap<String, IndexMap<String, String>>,
    #[serde(flatten)]
    pub extra: IndexMap<String, Value>,
}

impl Config {
    /// The config a package shows: the layer table's names and priorities joined with
    /// what the document says about each layer, hashtable paths under `hashes/`.
    pub fn from_package(m: &Metadata, table: &[Layer]) -> Config {
        let mut layers: Vec<&Layer> = table.iter().collect();
        layers.sort_by_key(|l| (l.name != BASE, l.priority, l.name.clone()));
        let layers = layers
            .into_iter()
            .map(|l| {
                let d = m.layer(&l.name);
                ConfigLayer {
                    name: l.name.clone(),
                    display_name: d.and_then(|d| d.display_name.clone()),
                    priority: l.priority,
                    description: d.and_then(|d| d.description.clone()),
                    string_overrides: d.map(|d| d.string_overrides.clone()).unwrap_or_default(),
                    extra: d.map(|d| d.extra.clone()).unwrap_or_default(),
                }
            })
            .collect();
        let hashtables = m
            .hashtables
            .iter()
            .filter_map(|h| {
                let file = h.path.strip_prefix(HASHES_PREFIX)?;
                (!file.is_empty() && !file.contains('/')).then(|| Hashtable {
                    path: format!("{HASHES}/{file}"),
                    ..h.clone()
                })
            })
            .collect();
        let mut extra = m.extra.clone();
        if let Some(d) = &m.distributor
            && let Ok(v) = serde_json::to_value(d)
        {
            extra.insert("distributor".into(), v);
        }
        Config {
            name: m.name.clone(),
            display_name: m.display_name.clone(),
            version: m.version.clone(),
            description: m.description.clone().unwrap_or_default(),
            authors: m
                .authors
                .iter()
                .map(|a| match &a.role {
                    Some(role) => ConfigAuthor::Role {
                        name: a.name.clone(),
                        role: role.clone(),
                    },
                    None => ConfigAuthor::Name(a.name.clone()),
                })
                .collect(),
            license: match &m.license {
                License::None | License::Unknown => None,
                License::Spdx { spdx_id } => Some(ConfigLicense::Spdx(spdx_id.clone())),
                License::Custom { name, url } => Some(ConfigLicense::Custom {
                    name: name.clone(),
                    url: (!url.is_empty()).then(|| url.clone()),
                }),
            },
            tags: m.tags.clone(),
            champions: m.champions.clone(),
            maps: m.maps.clone(),
            layers,
            thumbnail: None,
            hashtables,
            extra,
        }
    }

    /// Reads a config and checks what a package needs of it: a name, a display name
    /// and a version, every layer a slug, `base` at priority 0 if declared, every
    /// hashtable under `hashes/`. `INVALID_ARG` otherwise.
    pub fn parse(bytes: &[u8]) -> Result<Config> {
        let c: Config = serde_json::from_slice(bytes).map_err(|_| Error::INVALID_ARG)?;
        c.check()?;
        Ok(c)
    }

    /// What [`parse`](Self::parse) checks, for a config kept from a package.
    pub fn check(&self) -> Result<()> {
        let ok = !self.name.is_empty()
            && !self.display_name.is_empty()
            && !self.version.is_empty()
            && self.layers.iter().all(|l| path::is_slug(&l.name))
            && self
                .layers
                .iter()
                .all(|l| l.name != BASE || l.priority == 0)
            && self.hashtables.iter().all(|h| {
                h.path
                    .strip_prefix(HASHES_PREFIX_PROJECT)
                    .is_some_and(|f| !f.is_empty() && !f.contains('/'))
            });
        if ok { Ok(()) } else { Err(Error::INVALID_ARG) }
    }

    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).expect("a config of these types serializes")
    }

    /// The layer table: `base` first, then by priority and name.
    pub fn layer_table(&self) -> Vec<Layer> {
        let mut table: Vec<Layer> = self
            .layers
            .iter()
            .filter(|l| l.name != BASE)
            .map(|l| Layer {
                name: l.name.clone(),
                priority: l.priority,
            })
            .collect();
        table.sort_by(|a, b| (a.priority, &a.name).cmp(&(b.priority, &b.name)));
        table.dedup_by(|a, b| a.name == b.name);
        table.insert(
            0,
            Layer {
                name: BASE.into(),
                priority: 0,
            },
        );
        table
    }

    /// The document to write, for `table` (from [`layer_table`](Self::layer_table))
    /// and each layer's game-data document by name.
    pub fn to_metadata(&self, table: &[Layer], game_data: &IndexMap<String, Value>) -> Metadata {
        let mut extra = self.extra.clone();
        let distributor = extra
            .shift_remove("distributor")
            .and_then(|v| serde_json::from_value::<Distributor>(v).ok());
        Metadata {
            schema_version: Some(SCHEMA_VERSION),
            name: self.name.clone(),
            display_name: self.display_name.clone(),
            description: Some(self.description.clone()),
            version: self.version.clone(),
            distributor,
            authors: self
                .authors
                .iter()
                .map(|a| match a {
                    ConfigAuthor::Name(name) => Author {
                        name: name.clone(),
                        role: None,
                    },
                    ConfigAuthor::Role { name, role } => Author {
                        name: name.clone(),
                        role: Some(role.clone()),
                    },
                })
                .collect(),
            license: match &self.license {
                None => License::None,
                Some(ConfigLicense::Spdx(id)) => License::Spdx {
                    spdx_id: id.clone(),
                },
                Some(ConfigLicense::Custom { name, url }) => License::Custom {
                    name: name.clone(),
                    url: url.clone().unwrap_or_default(),
                },
            },
            tags: self.tags.clone(),
            champions: self.champions.clone(),
            maps: self.maps.clone(),
            layers: table
                .iter()
                .map(|l| {
                    let c = self.layers.iter().find(|c| c.name == l.name);
                    LayerMetadata {
                        name: l.name.clone(),
                        display_name: c.and_then(|c| c.display_name.clone()),
                        priority: l.priority,
                        description: c.and_then(|c| c.description.clone()),
                        string_overrides: c.map(|c| c.string_overrides.clone()).unwrap_or_default(),
                        game_data: game_data.get(&l.name).cloned(),
                        extra: c.map(|c| c.extra.clone()).unwrap_or_default(),
                    }
                })
                .collect(),
            hashtables: self
                .hashtables
                .iter()
                .map(|h| Hashtable {
                    path: format!(
                        "{HASHES_PREFIX}{}",
                        h.path
                            .strip_prefix(HASHES_PREFIX_PROJECT)
                            .unwrap_or(&h.path)
                    ),
                    ..h.clone()
                })
                .collect(),
            generator: Some(GENERATOR.into()),
            extra,
        }
    }
}

/// `hashes/`, where a project keeps its tables.
const HASHES_PREFIX_PROJECT: &str = "hashes/";

/// A layer's game-data document as its project manifest, `game_data.json`: the
/// document less each module's `origin`, which records the manifest it was packed
/// from and is the package's, not the project's.
pub fn game_data_manifest(document: &Value) -> Value {
    let mut v = document.clone();
    if let Some(modules) = v.get_mut("modules").and_then(Value::as_array_mut) {
        for m in modules.iter_mut().filter_map(Value::as_object_mut) {
            m.shift_remove("origin");
        }
    }
    v
}

/// A project manifest as the document a package stores: each module gets the
/// `origin` the reference packer gives it, unless it has one.
pub fn game_data_document(mut manifest: Value) -> Value {
    if let Some(modules) = manifest.get_mut("modules").and_then(Value::as_array_mut) {
        for (i, m) in modules.iter_mut().enumerate() {
            if let Some(m) = m.as_object_mut()
                && !m.contains_key("origin")
            {
                m.insert(
                    "origin".into(),
                    serde_json::json!({ "manifest": path::GAME_DATA, "source": null, "module": i }),
                );
            }
        }
    }
    manifest
}
