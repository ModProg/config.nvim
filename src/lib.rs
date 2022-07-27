use std::{
    collections::HashMap,
    env,
    fs::{self, FileType},
    path::{Path, PathBuf},
};

use derive_more::Display;
use merge::Merge;
use mlua::prelude::*;
use nvim::{
    api::{
        Callback, Mode, NvimCreateAutocmdsOpts, NvimCreateAutocmdsOptsBuilder, NvimSetKeymapOpts,
    },
    common::Index,
    fns::PathType,
    InputOpts, Vim,
};
use serde::Deserialize;
use serde_with::{flattened_maybe, serde_as, FromInto, OneOrMany};
use sha2::{Digest, Sha512};
use smart_default::SmartDefault;
use walkdir::WalkDir;

#[serde_as]
#[derive(Debug, Deserialize, SmartDefault, Clone)]
#[serde(default)]
struct Keys {
    #[serde_as(deserialize_as = "OneOrMany<_>")]
    modes: Vec<Mode>,
    #[default = true]
    recursive: bool,
    command: bool,
    silent: bool,
    unique: bool,
    expression: bool,
    leader: String,
    // TODO file_type: String,
    #[serde(flatten)]
    mappings_: HashMap<String, String>,
    mappings: HashMap<String, String>,
}
flattened_maybe!(deserialize_mappings, "mappings");

#[derive(Debug, Deserialize, Clone)]
struct Set(String, Operation, SetValue);

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SetDeserializer {
    Flag(String),
    Assignment(HashMap<String, ValueOrOp>),
}

impl From<SetDeserializer> for Vec<Set> {
    fn from(d: SetDeserializer) -> Self {
        match d {
            SetDeserializer::Flag(name) => {
                vec![if let Some(name) = name.strip_prefix("no") {
                    Set(name.to_string(), Operation::Assign, SetValue::Bool(false))
                } else {
                    Set(name.to_string(), Operation::Assign, SetValue::Bool(true))
                }]
            }
            SetDeserializer::Assignment(map) => map
                .into_iter()
                .flat_map(|(name, value)| match value {
                    ValueOrOp::Operation(map) => map
                        .into_iter()
                        .map(|(operation, value)| Set(name.clone(), operation, value))
                        .collect(),
                    ValueOrOp::Value(value) => vec![Set(name, Operation::Assign, value)],
                })
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SetsDeserializer {
    List(Vec<SetDeserializer>),
    Map {
        #[serde(default)]
        flags: Vec<SetDeserializer>,
        #[serde(default, flatten)]
        map: HashMap<String, ValueOrOp>,
    },
}

impl From<SetsDeserializer> for Vec<Set> {
    fn from(d: SetsDeserializer) -> Self {
        match d {
            SetsDeserializer::List(list) => list.into_iter().flat_map(Vec::from).collect(),
            SetsDeserializer::Map { flags, map } => flags
                .into_iter()
                .flat_map(Vec::from)
                .chain(map.into_iter().flat_map(|(name, value)| {
                    match value {
                        ValueOrOp::Operation(map) => map
                            .into_iter()
                            .map(|(operation, value)| Set(name.clone(), operation, value))
                            .collect(),
                        ValueOrOp::Value(value) => vec![Set(name, Operation::Assign, value)],
                    }
                }))
                .collect(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
enum ValueOrOp {
    Operation(HashMap<Operation, SetValue>),
    Value(SetValue),
}

#[derive(Debug, Deserialize, ToLua, Clone)]
#[serde(untagged)]
enum SetValue {
    Bool(bool),
    String(String),
    List(Vec<SetValue>),
    Map(HashMap<String, SetValue>),
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, Display, Clone, Copy)]
enum Operation {
    #[serde(alias = "+", alias = "append")]
    #[display(fmt = "appending")]
    Append,
    #[serde(alias = "^", alias = "prepend")]
    #[display(fmt = "prepending")]
    Prepend,
    #[serde(alias = "-", alias = "remove")]
    #[display(fmt = "remove")]
    Remove,
    #[serde(alias = "value", alias = "=", alias = "assign")]
    #[display(fmt = "assigning")]
    Assign,
}

#[serde_as]
#[derive(Debug, Deserialize, Default, PartialEq, Hash, Eq, Clone)]
#[serde(default)]
struct Condition {
    #[serde(default)]
    #[serde_as(deserialize_as = "OneOrMany<_>")]
    filetype: Vec<String>,
}

impl Condition {
    fn events(&self) -> Vec<String> {
        vec!["FileType".to_string()]
    }
    fn opts(&self) -> NvimCreateAutocmdsOptsBuilder {
        NvimCreateAutocmdsOpts::builder()
            // .group(StrI64::String(String::from("Config")))
            .pattern(self.filetype.clone())
            .to_owned()
    }
}

impl IntoIterator for Condition {
    type Item = Condition;

    type IntoIter = <Vec<Condition> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.filetype
            .into_iter()
            .map(|filetype| Condition {
                filetype: vec![filetype],
            })
            .collect::<Vec<_>>()
            .into_iter()
    }
}

#[serde_as]
#[derive(Debug, Deserialize, Default, Merge, Clone)]
#[serde(default)]
struct Config {
    #[merge(skip)]
    conditions: Vec<Condition>,
    #[merge(strategy = merge::vec::append)]
    keys: Vec<Keys>,
    #[merge(strategy = merge::vec::append)]
    #[serde_as(deserialize_as = "FromInto<SetsDeserializer>")]
    set: Vec<Set>,
    // TODO let
}

impl Config {
    fn merge_into_hashmap(self, hash_map: &mut HashMap<Condition, Self>) {
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
}

impl Config {
    fn apply(&self, vim: &Vim, buffer: bool) {
        for Keys {
            modes,
            recursive,
            command,
            mappings,
            mappings_,
            silent,
            unique,
            expression,
            leader,
        } in &self.keys
        {
            for mode in modes {
                for (lhs, rhs) in mappings.iter().chain(mappings_.iter()) {
                    let cmd;
                    let set_keymap: &fn(&Vim, Mode, &str, &str, NvimSetKeymapOpts) = &if buffer {
                        |vim, a, b, c, d| vim.api().nvim_buf_set_keymap(Index::Current, a, b, c, d)
                    } else {
                        |vim, a, b, c, d| vim.api().nvim_set_keymap(a, b, c, d)
                    };

                    set_keymap(
                        vim,
                        *mode,
                        &(leader.clone() + lhs),
                        if *command {
                            cmd = format!("<CMD>{rhs}<CR>");
                            &cmd
                        } else {
                            rhs
                        },
                        NvimSetKeymapOpts::builder()
                            .noremap(!recursive)
                            .silent(*silent)
                            .unique(*unique)
                            .expr(*expression)
                            .build(),
                    )
                }
            }
        }
        for Set(key, op, value) in &self.set {
            let opt = if buffer { vim.opt_local() } else { vim.opt() };
            if let Err(err) = match op {
                Operation::Append => opt.append(key, value.clone()),
                Operation::Prepend => opt.prepend(key, value.clone()),
                Operation::Remove => opt.remove(key, value.clone()),
                Operation::Assign => opt.set(key, value.clone()),
            } {
                vim.notify(
                    &format!("Error while {op} {value:?} to {key}: \n{err}"),
                    nvim::LogLevel::Error,
                    None,
                )
            }
        }
    }
}

#[derive(Deserialize, Debug, Default, Clone)]
struct Hashes(HashMap<PathBuf, Vec<u8>>);
impl Hashes {
    fn is_hashed(&self, path: &Path, config: &str) -> bool {
        if let Some(hash) = self.0.get(path) {
            let mut hasher = Sha512::new();
            hasher.update(config);
            let result = hasher.finalize();
            hash == result.as_slice()
        } else {
            false
        }
    }
    fn add_hash(&mut self, path: PathBuf, config: &str) {
        self.0.insert(path, {
            let mut hasher = Sha512::new();
            hasher.update(config);
            hasher.finalize().to_vec()
        });
    }
}

fn config_files(path: &Path) -> impl Iterator<Item = (PathBuf, String, Config)> {
    WalkDir::new(path)
        .into_iter()
        .filter_map(|path| match path {
            Ok(path) if path.file_type().is_file() => match path.path().extension()?.to_str()? {
                "toml" => {
                    let string = fs::read_to_string(path.path()).ok()?;
                    let config = toml::from_str(&string).ok()?;
                    Some((path.into_path(), string, config))
                }
                "yaml" | "yml" | "json" => {
                    let string = fs::read_to_string(path.path()).ok()?;
                    let config = serde_yaml::from_str(&string).unwrap();
                    Some((path.into_path(), string, config))
                }
                _ => None,
            },
            _ => None,
        })
}

fn unhashed_files<'a>(
    hashes: &'a Hashes,
    files: impl IntoIterator<Item = &'a (PathBuf, String, Config)>,
) -> impl Iterator<Item = &'a Path> {
    files.into_iter().filter_map(|(path, string, config)| {
        if hashes.is_hashed(path, string) {
            None
        } else {
            Some(path.as_path())
        }
    })
}

fn get_config_dirs() -> Vec<PathBuf> {
    let mut nvim_folders = Vec::new();
    let mut cwd: &Path = &env::current_dir().unwrap();
    let nvim_dir = cwd.join(".nvim/config");
    if nvim_dir.is_dir() {
        nvim_folders.push(nvim_dir)
    }
    while let Some(parent) = cwd.parent() {
        cwd = parent;
        let nvim_dir = cwd.join(".nvim/config");
        if nvim_dir.is_dir() {
            nvim_folders.push(nvim_dir)
        }
    }
    nvim_folders.reverse();
    nvim_folders
}

fn load_config(lua: &Lua, _: ()) -> LuaResult<()> {
    let vim = &Vim::from(lua);
    let get_files = |pattern| vim.api().nvim_get_runtime_file(pattern, true).into_iter();

    let mut conditional_configs: HashMap<Condition, Config> = HashMap::new();

    for path in get_files("config/*/*.toml") {
        let config: Config = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        config.merge_into_hashmap(&mut conditional_configs);
    }
    for path in get_files("config/*/*.yml")
        .chain(get_files("config/*/*.yaml"))
        .chain(get_files("config/*/*.json"))
    {
        let config: Config = serde_yaml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        config.merge_into_hashmap(&mut conditional_configs);
    }

    let hashes = PathBuf::from(vim.fns().stdpath(PathType::Data)).join("config/hashes");

    let hashes = || -> Option<Hashes> { rmp_serde::from_slice(&fs::read(hashes).ok()?).ok()? }()
        .unwrap_or_default();

    let local_configs = get_config_dirs();
    let config_files: Vec<_> = get_config_dirs()
        .iter()
        .flat_map(|path| config_files(path))
        .collect();
    let unknown = unhashed_files(&hashes, &config_files);
    let fun = lua.create_function(|lua, s: String| {
        Vim::from(lua).notify(&format!("Hi: {s}"), nvim::LogLevel::Error, None);
        Ok(())
    })?;
    vim.ui().input(InputOpts::builder().build(), fun);

    // TODO diff config
    if let Some(config) = conditional_configs.remove(&Condition::default()) {
        config.apply(vim, false);
    }
    for (condition, config) in conditional_configs {
        vim.api().nvim_create_autocmd(
            condition.events(),
            condition
                .opts()
                .callback(Callback::from_fn(lua, move |lua: &Lua, _| {
                    Vim::from(lua).notify(&format!("{config:?}"), nvim::LogLevel::Warn, None);
                    config.apply(&Vim::from(lua), true);
                    false
                }))
                .build(),
        );
    }
    Ok(())
}

#[mlua::lua_module]
fn config(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;
    exports.set("load_config", lua.create_function(load_config)?)?;
    Ok(exports)
}
