#![warn(clippy::pedantic)]
#![allow(
    clippy::struct_excessive_bools,
    clippy::too_many_lines,
    clippy::unnecessary_wraps
)]
#![warn(clippy::unwrap_used)]
use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
};

use derive_more::Display;
use itertools::{Either, Itertools};
use merge::Merge;
use nvim_oxi::{self as oxi, Error};
use oxi::{
    api::{self, Buffer, Window},
    object,
    opts::{
        CreateAutocmdOpts, CreateAutocmdOptsBuilder, OptionScope, OptionValueOpts, SetKeymapOpts,
    },
    types::{LogLevel, Mode, OptionInfos},
    Dictionary, Function, Object, ToObject,
};
use serde::{Deserialize, Serialize};
use serde_with::{flattened_maybe, serde_as, FromInto, OneOrMany};
use sha2::{Digest, Sha512};
use smart_default::SmartDefault;
use walkdir::WalkDir;

type Result<T, E = Error> = std::result::Result<T, E>;

macro_rules! continue_on_error {
    ($expr:expr, $err:ident, $($format:tt)*) => {
        match $expr {
            Ok(value) => value,
            Err($err) => {
                api::notify(&format!($($format)*), LogLevel::Error, None).unwrap();
                continue;
            }
        }
    };
    ($expr:expr, $($format:tt)*) => {
        if let Ok(value) = $expr {
             value
        } else {
            api::notify(&format!($($format)*), LogLevel::Error, None).unwrap();
            continue;
        }
    };
}
macro_rules! log_error {
    ($($format:tt)*) => {
        api::notify(&format!($($format)*), LogLevel::Error, None).unwrap();
    };
}

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

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
enum SetValue {
    Bool(bool),
    String(String),
    Integer(i64),
    Float(f64),
    List(Vec<String>),
    Set(HashSet<char>),
    Map(HashMap<String, String>),
}

impl SetValue {
    fn from_option(commalist: bool, flaglist: bool, name: &str, object: Object) -> Result<Self> {
        let object_kind = object.kind();
        let deserializer = object::Deserializer::new(object);
        if commalist {
            let s = String::deserialize(deserializer)?;
            let s: Vec<_> = s.split(',').map(String::from).collect();
            // TODO https://github.com/neovim/neovim/issues/19574
            // Hardcode for now from: https://github.com/neovim/neovim/blob/e59bc078de624a5f3220bfd2713be3f8978c5672/runtime/lua/vim/_meta.lua#L199-L203
            if matches!(name, "fillchars" | "listchars" | "winhl") {
                Ok(Self::Map(
                    s.into_iter()
                        .filter(|s|!s.is_empty())
                        .filter_map(|entry| {
                            entry
                                .split_once(':')
                                .map(|(a, b)| (a.to_owned(), b.to_owned()))
                                .or_else(|| {
                                    log_error!(
                                        "{name} should only contain map entries, contained `{entry}`"
                                    );
                                    None
                                })
                        })
                        .collect(),
                ))
            } else {
                Ok(Self::List(s))
            }
        } else if flaglist {
            let s = String::deserialize(deserializer)?;
            Ok(Self::Set(s.chars().collect()))
        } else {
            match object_kind {
                oxi::ObjectKind::Boolean => Ok(Self::Bool(Deserialize::deserialize(deserializer)?)),
                oxi::ObjectKind::Float => Ok(Self::Float(Deserialize::deserialize(deserializer)?)),
                oxi::ObjectKind::Integer => Ok(Self::Integer(Deserialize::deserialize(deserializer)?)),
                kind => Err(Error::DeserializeError(format!(
                    "{name} should be of kind string, boolean or float not {kind:?}"
                ))),
            }
        }
    }
}

impl ToObject for SetValue {
    fn to_obj(self) -> Result<Object> {
        match self {
            SetValue::List(v) => v.join(",").to_obj(),
            SetValue::Map(v) => v
                .into_iter()
                .map(|(k, v)| format!("{k}:{v}"))
                .collect::<Vec<_>>()
                .join(",")
                .to_obj(),
            SetValue::Set(v) => v.into_iter().collect::<String>().to_obj(),
            v => v.serialize(object::Serializer::new()),
        }
    }
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
        let mut ret = Vec::new();
        if !self.filetype.is_empty() {
            ret.push("FileType".to_string());
        }
        ret
    }
    fn opts(&self) -> CreateAutocmdOptsBuilder {
        CreateAutocmdOpts::builder()
            // .group(StrI64::String(String::from("Config")))
            .patterns(self.filetype.iter().map(AsRef::as_ref))
            .clone()
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

    fn load(path: &Path) -> Result<(Self, String), String> {
        let ext = path
            .extension()
            .expect("files matching glob have extension");
        let file = fs::read_to_string(&path)
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

    fn apply(&self, buffer: bool) -> Result<()> {
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
                    let set_keymap: &fn(Mode, &str, &str, SetKeymapOpts) -> Result<()> = &if buffer
                    {
                        |a, b, c, d| Buffer::current().set_keymap(a, b, c, Some(&d))
                    } else {
                        |a, b, c, d| api::set_keymap(a, b, c, Some(&d))
                    };

                    set_keymap(
                        *mode,
                        &(leader.clone() + lhs),
                        if *command {
                            cmd = format!("<CMD>{rhs}<CR>");
                            &cmd
                        } else {
                            rhs
                        },
                        SetKeymapOpts::builder()
                            .noremap(!recursive)
                            .silent(*silent)
                            .unique(*unique)
                            .expr(*expression)
                            .build(),
                    )?;
                }
            }
        }
        for Set(key, op, value) in &self.set {
            let OptionInfos {
                commalist,
                flaglist,
                name,
                scope,
                ..
            } = continue_on_error!(
                api::get_option_info(key),
                error,
                "Invalid option {key}: {error}"
            );

            let set_option: fn(name: &str, value: SetValue) -> Result<()> = match scope {
                oxi::types::OptionScope::Buffer => {
                    |name, value| Buffer::current().set_option(name, value)
                }
                oxi::types::OptionScope::Global if buffer => |name, value: SetValue| {
                    api::set_option_value(
                        name,
                        value,
                        Some(&OptionValueOpts::builder().scope(OptionScope::Local).build()),
                    )
                },
                oxi::types::OptionScope::Global => |name, value: SetValue| {
                    api::set_option_value(
                        name,
                        value,
                        Some(
                            &OptionValueOpts::builder()
                                .scope(OptionScope::Global)
                                .build(),
                        ),
                    )
                },
                oxi::types::OptionScope::Window => {
                    |name, value| Window::current().set_option(name, value)
                }
                _ => return Err(Error::Other(format!("Unsuported Option scope: {scope:?}"))),
            };

            let get_option: fn(name: &str) -> Result<Object> = match scope {
                oxi::types::OptionScope::Buffer => |name| Buffer::current().get_option(name),
                oxi::types::OptionScope::Global if buffer => |name| {
                    api::get_option_value(
                        name,
                        Some(&OptionValueOpts::builder().scope(OptionScope::Local).build()),
                    )
                },
                oxi::types::OptionScope::Global => |name| {
                    api::get_option_value(
                        name,
                        Some(
                            &OptionValueOpts::builder()
                                .scope(OptionScope::Global)
                                .build(),
                        ),
                    )
                },
                oxi::types::OptionScope::Window => {
                    |name| -> Result<Object, Error> { Window::current().get_option(name) }
                }
                _ => return Err(Error::Other(format!("Unsuported Option scope: {scope:?}"))),
            };

            let key = &key;
            let current = SetValue::from_option(commalist, flaglist, &name, get_option(key)?)?;

            if let Err(err) = match (current, value.clone(), op) {
                (SetValue::Set(_), SetValue::List(value), Operation::Assign) => set_option(
                    key,
                    SetValue::Set(value.iter().flat_map(|s| s.chars()).collect()),
                ),
                (_, value, Operation::Assign) => set_option(key, value),
                (SetValue::Float(current), SetValue::Float(value), Operation::Append) => {
                    set_option(key, SetValue::Float(current + value))
                }
                (SetValue::Float(current), SetValue::Float(value), Operation::Remove) => {
                    set_option(key, SetValue::Float(value - current))
                }
                (SetValue::Integer(current), SetValue::Integer(value), Operation::Append) => {
                    set_option(key, SetValue::Integer(current + value))
                }
                (SetValue::Integer(current), SetValue::Integer(value), Operation::Remove) => {
                    set_option(key, SetValue::Integer(value - current))
                }
                (SetValue::String(current), SetValue::String(value), Operation::Append) => {
                    set_option(key, SetValue::String(current + &value))
                }
                (SetValue::String(current), SetValue::String(value), Operation::Prepend) => {
                    set_option(key, SetValue::String(value + &current))
                }
                (SetValue::String(current), SetValue::String(value), Operation::Remove) => {
                    set_option(key, SetValue::String(current.replacen(&value, "", 1)))
                }
                (SetValue::List(mut current), SetValue::List(mut value), Operation::Append) => {
                    current.append(&mut value);
                    set_option(key, SetValue::List(current))
                }
                (SetValue::List(mut current), SetValue::List(mut value), Operation::Prepend) => {
                    value.append(&mut current);
                    set_option(key, SetValue::List(current))
                }
                (SetValue::List(mut current), SetValue::List(values), Operation::Remove) => {
                    for value in values {
                        if let Some(index) = current.iter().position(|v| v == &value) {
                            current.remove(index);
                        }
                    }
                    set_option(key, SetValue::List(current))
                }
                (SetValue::List(mut current), SetValue::String(value), Operation::Append) => {
                    current.push(value);
                    set_option(key, SetValue::List(current))
                }
                (SetValue::List(mut current), SetValue::String(value), Operation::Prepend) => {
                    current.insert(0, value);
                    set_option(key, SetValue::List(current))
                }
                (SetValue::List(mut current), SetValue::String(value), Operation::Remove) => {
                    if let Some(index) = current.iter().position(|v| v == &value) {
                        current.remove(index);
                    }
                    set_option(key, SetValue::List(current))
                }
                (
                    SetValue::Set(mut current),
                    SetValue::String(value),
                    Operation::Append | Operation::Prepend,
                ) => {
                    current.extend(value.chars());
                    set_option(key, SetValue::Set(current))
                }
                (SetValue::Set(mut current), SetValue::String(value), Operation::Remove) => {
                    current.retain(|&v| !value.contains(v));
                    set_option(key, SetValue::Set(current))
                }
                (
                    SetValue::Set(mut current),
                    SetValue::List(value),
                    Operation::Append | Operation::Prepend,
                ) => {
                    current.extend(value.iter().flat_map(|s| s.chars()));
                    set_option(key, SetValue::Set(current))
                }
                (SetValue::Set(mut current), SetValue::List(values), Operation::Remove) => {
                    for value in values {
                        current.retain(|&v| !value.contains(v));
                    }
                    set_option(key, SetValue::Set(current))
                }
                (
                    SetValue::Map(mut current),
                    SetValue::Map(value),
                    Operation::Append | Operation::Prepend,
                ) => {
                    current.extend(value.into_iter());
                    set_option(key, SetValue::Map(current))
                }
                (SetValue::Map(mut current), SetValue::List(values), Operation::Remove) => {
                    for value in values {
                        current.remove(&value);
                    }
                    set_option(key, SetValue::Map(current))
                }
                (SetValue::Map(mut current), SetValue::String(value), Operation::Remove) => {
                    current.remove(&value);
                    set_option(key, SetValue::Map(current))
                }
                (current, value, op) => {
                    api::notify(
                        &format!("{op} {value:?} to {current:?} of {key} is not supported"),
                        LogLevel::Error,
                        None,
                    )?;
                    continue;
                }
            } {
                api::notify(
                    &format!("Error while {op} {value:?} to {key}: \n{err}"),
                    LogLevel::Error,
                    None,
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
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
                    let config = serde_yaml::from_str(&string).ok()?;
                    Some((path.into_path(), string, config))
                }
                _ => None,
            },
            _ => None,
        })
}

fn unhashed_files(
    hashes: &Hashes,
    files: impl IntoIterator<Item = (PathBuf, String, Config)>,
) -> (Vec<PathBuf>, Vec<Config>) {
    files.into_iter().partition_map(|(path, string, config)| {
        if hashes.is_hashed(&path, &string) {
            Either::Right(config)
        } else {
            Either::Left(path)
        }
    })
}

fn get_config_dirs() -> Vec<PathBuf> {
    let mut nvim_folders = Vec::new();
    let cwd = match env::current_dir() {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };
    let mut cwd = cwd.as_path();
    let nvim_dir = cwd.join(".nvim/config");
    if nvim_dir.is_dir() {
        nvim_folders.push(nvim_dir);
    }
    while let Some(parent) = cwd.parent() {
        cwd = parent;
        let nvim_dir = cwd.join(".nvim/config");
        if nvim_dir.is_dir() {
            nvim_folders.push(nvim_dir);
        }
    }
    nvim_folders.reverse();
    nvim_folders
}

fn load_config(_: ()) -> Result<()> {
    let get_files = |pattern| api::get_runtime_file(pattern, true);

    let mut conditional_configs: HashMap<Condition, Config> = HashMap::new();

    for path in get_files("config/*.yml")?
        .chain(get_files("config/*.yaml")?)
        .chain(get_files("config/*.json")?)
        .chain(get_files("config/*.toml")?)
    {
        continue_on_error!(Config::load(path.as_path()), error, "{error}")
            .0
            .merge_into_hashmap(&mut conditional_configs);
    }

    let stdpath: String =
        api::call_function("stdpath", ("data",)).expect("There is a stdpath for data");
    let hashes_file = PathBuf::from(stdpath).join("config/hashes");

    let mut hashes =
        || -> Option<Hashes> { rmp_serde::from_slice(&fs::read(&hashes_file).ok()?).ok()? }()
            .unwrap_or_default();

    let config_files: Vec<_> = get_config_dirs()
        .iter()
        .flat_map(|path| config_files(path))
        .collect();
    let (unknown, known) = unhashed_files(&hashes, config_files);
    for config in known {
        config.merge_into_hashmap(&mut conditional_configs);
    }
    if !unknown.is_empty() {
        {
            let unknown: Vec<_> = unknown.iter().map(|p| p.to_string_lossy()).collect();
            api::notify(
                &format!(
                    "Found new local config: \n  {}\nRun :ConfigAllow to activate",
                    unknown.join("\n  ")
                ),
                LogLevel::Info,
                None,
            )?;
        }
        api::create_user_command(
            "ConfigAllow",
            move |_| {
                for file in &unknown {
                    let (config, source) = continue_on_error!(Config::load(file), error, "{error}");
                    config.apply(false)?;
                    hashes.add_hash(file.clone(), &source);
                }
                let data_dir = hashes_file.parent().expect("Hashes file has a parent");
                fs::create_dir_all(data_dir).map_err(|e| {
                    Error::Other(format!(
                        "Error while creating data dir `{}`: {e}",
                        data_dir.display()
                    ))
                })?;
                fs::write(
                    &hashes_file,
                    rmp_serde::to_vec(&hashes).expect("Hashes serialization is infallible"),
                )
                .map_err(|e| {
                    Error::Other(format!(
                        "Error while saving hashes `{}`: {e}",
                        hashes_file.display()
                    ))
                })?;
                Ok(())
            },
            None,
        )?;
    }

    if let Some(config) = conditional_configs.remove(&Condition::default()) {
        config.apply(false)?;
    }
    for (condition, config) in conditional_configs {
        api::create_autocmd(
            condition.events().iter().map(AsRef::as_ref),
            &condition
                .opts()
                .callback(move |_| {
                    config.apply(true)?;
                    Ok(false)
                })
                .build(),
        )
        .expect("Create autocommand for conditional config");
    }
    Ok(())
}

#[oxi::module]
fn config() -> Result<Dictionary> {
    Ok(Dictionary::from_iter([(
        "load_config",
        Function::from_fn(load_config),
    )]))
}
