use std::{collections::HashMap, fs, path::Path};

use merge::Merge;
use oxi::{self as oxi, api::create_autocmd};
use serde::Deserialize;
use serde_with::{serde_as, FromInto, OneOrMany};

mod condition;
pub use condition::*;
mod set;
pub use set::*;
mod keys;
pub use keys::*;

use crate::*;

#[serde_as]
#[derive(Deserialize, Debug, Clone)]
pub struct AutoCommand {
    #[serde_as(deserialize_as = "OneOrMany<_>")]
    triggers: Vec<String>,
    #[serde(default)]
    #[serde_as(deserialize_as = "OneOrMany<_>")]
    cmd: Vec<String>,
    #[serde(default)]
    #[serde_as(deserialize_as = "OneOrMany<_>")]
    lua: Vec<String>,
    pattern: Option<String>,
    // #[serde(default)]
    // event: HashMap<String, String>,
    // #[serde(default)]
    // silent: bool,
}

#[serde_as]
#[derive(Debug, Deserialize, Default, Merge, Clone)]
#[serde(default)]
pub struct Config {
    // TODO investigate if `or` is the right way to interpret multiple conditions
    #[merge(skip)]
    pub conditions: Vec<Condition>,
    #[merge(strategy = merge::vec::append)]
    pub keys: Vec<Keys>,
    #[merge(strategy = merge::vec::append)]
    #[serde_as(deserialize_as = "FromInto<SetsDeserializer>")]
    pub set: Vec<Set>,
    #[merge(strategy = merge::vec::append)]
    #[serde_as(deserialize_as = "OneOrMany<_>")]
    pub auto_commands: Vec<AutoCommand>,
}

impl Config {
    pub fn merge_into_hashmap(self, hash_map: &mut HashMap<Condition, Self>) {
        let mut conditions = self
            .conditions
            .clone()
            .into_iter()
            .flat_map(IntoIterator::into_iter)
            .peekable();
        if conditions.peek().is_none() {
            if let Some(config) = hash_map.get_mut(&Condition::default()) {
                config.merge(self);
            } else {
                hash_map.insert(Condition::default(), self);
            }
        } else {
            for condition in conditions {
                if let Some(config) = hash_map.get_mut(&condition) {
                    config.merge(self.clone());
                } else {
                    hash_map.insert(condition, self.clone());
                }
            }
        }
    }

    pub fn load(path: &Path) -> Result<(Self, String), String> {
        let ext = path
            .extension()
            .expect("files matching glob have extension");
        let file = fs::read_to_string(path)
            .map_err(|error| format!("error while reading {}: {error}", path.display()))?;

        Ok((
            match ext.to_string_lossy().to_ascii_lowercase().as_str() {
                "json" | "yml" | "yaml" => serde_yaml::from_str(&file).map_err(|e| e.to_string()),
                "toml" => toml::from_str(&file).map_err(|e| e.to_string()),
                _ => unreachable!("files matching glob are handled"),
            }
            .map_err(|error| format!("error while parsing {}: {error}", path.display()))?,
            file,
        ))
    }

    pub fn apply(&self, buffer: bool) -> ApiResult {
        for key in &self.keys {
            key.apply(buffer)?;
        }
        for set in &self.set {
            set.apply(buffer)?;
        }
        for AutoCommand {
            triggers,
            cmd,
            lua,
            pattern,
        } in &self.auto_commands
        {
            for cmd in cmd.clone().into_iter().chain(
                lua.iter()
                    .map(|lua| format!("lua {lua}{}", if lua.ends_with(')') { "" } else { "()" })),
            ) {
                create_autocmd(
                    triggers.iter().map(AsRef::as_ref),
                    &CreateAutocmdOpts::builder()
                        .patterns(pattern.iter().map(AsRef::as_ref))
                        .command(cmd.as_str())
                        .build(),
                )?;
            }
        }
        Ok(())
    }
}
