//! [`Resolver`]: path hashes for names that need the tables to read back.

use crate::hash::PathHash;
use crate::names::{open, verified};
use lol7z_api::Result;
use ltk_hashdb::HashDb;
use ltk_mimir_cache::{HashStore, Table};
use std::path::Path;

/// Reads names back as [`PathHash::from_path`] does, and also a too-long name moved or
/// renamed: `preamble«FULL-OUTER».ext` whose path no longer hashes to OUTER is FULL's
/// table path with the new folder, preamble and extension around its hidden middle.
pub struct Resolver {
    store: Option<HashStore>,
    /// Opened the first time a name needs them.
    dbs: Option<Vec<HashDb>>,
}

impl Resolver {
    /// With the machine's shared tables.
    pub fn new() -> Self {
        Self {
            store: HashStore::discover().ok(),
            dbs: None,
        }
    }

    /// With the tables in `dir`, laid out as the shared cache is.
    pub fn with_dir(dir: &Path) -> Self {
        Self {
            store: Some(HashStore::at(dir)),
            dbs: None,
        }
    }

    /// `INVALID_ARG` where [`PathHash::from_path`] fails, unless the tables know FULL.
    pub fn hash(&mut self, path: &str) -> Result<PathHash> {
        PathHash::read(path, |full| self.lookup(full))
    }

    fn lookup(&mut self, hash: PathHash) -> Option<String> {
        let store = self.store.as_ref()?;
        let dbs = self.dbs.get_or_insert_with(|| {
            [Table::Game, Table::Lcu]
                .into_iter()
                .filter_map(|t| open(store, t).ok().map(|(db, _)| db))
                .collect()
        });
        dbs.iter().find_map(|db| {
            let path = db.get(hash.0)?;
            let path = path.as_str();
            verified(path, hash).then(|| path.to_owned())
        })
    }
}

impl Default for Resolver {
    fn default() -> Self {
        Self::new()
    }
}
