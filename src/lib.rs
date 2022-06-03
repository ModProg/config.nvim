use std::{collections::HashMap, fs};

use derive_more::Display;
use merge::Merge;
use mlua::prelude::*;
use nvim::{
    api::{Mode, NvimSetKeymapOpts},
    Vim,
};
use serde::Deserialize;
use serde_with::{flattened_maybe, serde_as, FromInto, OneOrMany};
use smart_default::SmartDefault;

#[serde_as]
#[derive(Debug, Deserialize, SmartDefault)]
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

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, Display)]
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
#[derive(Debug, Deserialize, Default, Merge)]
#[serde(default)]
struct Config {
    #[merge(strategy = merge::vec::append)]
    keys: Vec<Keys>,
    #[merge(strategy = merge::vec::append)]
    #[serde_as(deserialize_as = "FromInto<SetsDeserializer>")]
    set: Vec<Set>,
}

impl Config {
    fn apply(&self, vim: &Vim) {
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
                    vim.api().nvim_set_keymap(
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
            if let Err(err) = match op {
                Operation::Append => vim.opt().append(key, value.clone()),
                Operation::Prepend => vim.opt().prepend(key, value.clone()),
                Operation::Remove => vim.opt().remove(key, value.clone()),
                Operation::Assign => vim.opt().set(key, value.clone()),
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

fn load_config(lua: &Lua, _: ()) -> LuaResult<()> {
    let vim = &Vim::from(lua);
    let get_files = |pattern| vim.api().nvim_get_runtime_file(pattern, true).into_iter();
    let mut config = Config::default();

    for path in get_files("config/*.toml") {
        config.merge(toml::from_str(&fs::read_to_string(path).unwrap()).unwrap());
    }
    for path in get_files("config/*.yml").chain(get_files("config/*.yaml")).chain(get_files("config/*.json")) {
        config.merge(serde_yaml::from_str(&fs::read_to_string(path).unwrap()).unwrap());
    }

    // TODO diff config
    config.apply(vim);
    Ok(())
}

#[mlua::lua_module]
fn config(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;
    exports.set("load_config", lua.create_function(load_config)?)?;
    Ok(exports)
}
