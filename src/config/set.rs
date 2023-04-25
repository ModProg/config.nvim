use std::collections::{HashMap, HashSet};

use derive_more::Display;
use oxi::{
    api::{
        types::{self, OptionInfos},
        Buffer, Window,
    },
    conversion::{self, ToObject},
    Object, ObjectKind,
};
use serde::{Deserialize, Serialize};

use crate::*;

#[derive(Debug, Deserialize, Clone)]
pub struct Set(pub String, pub Operation, pub SetValue);

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum SetDeserializer {
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
pub enum SetsDeserializer {
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
pub enum ValueOrOp {
    Operation(HashMap<Operation, SetValue>),
    Value(SetValue),
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(untagged)]
pub enum SetValue {
    Bool(bool),
    String(String),
    Integer(i64),
    Float(f64),
    List(Vec<String>),
    Set(HashSet<char>),
    Map(HashMap<String, String>),
}

#[derive(Debug, Deserialize, PartialEq, Eq, Hash, Display, Clone, Copy)]
pub enum Operation {
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

impl SetValue {
    pub fn from_option(
        commalist: bool,
        flaglist: bool,
        name: &str,
        object: Object,
    ) -> ConvResult<Self> {
        let object_kind = object.kind();
        let deserializer = nvim_oxi::serde::Deserializer::new(object);
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
                ObjectKind::Boolean => Ok(Self::Bool(Deserialize::deserialize(deserializer)?)),
                ObjectKind::Float => Ok(Self::Float(Deserialize::deserialize(deserializer)?)),
                ObjectKind::Integer => Ok(Self::Integer(Deserialize::deserialize(deserializer)?)),
                kind => Err(conversion::Error::FromWrongType {
                    expected: "string, boolean or float",
                    actual: kind.as_static(),
                }),
            }
        }
    }
}

impl ToObject for SetValue {
    fn to_object(self) -> Result<nvim_oxi::Object, conversion::Error> {
        match self {
            SetValue::List(v) => v.join(",").to_object(),
            SetValue::Map(v) => v
                .into_iter()
                .map(|(k, v)| format!("{k}:{v}"))
                .collect::<Vec<_>>()
                .join(",")
                .to_object(),
            SetValue::Set(v) => v.into_iter().collect::<String>().to_object(),
            v => v
                .serialize(nvim_oxi::serde::Serializer::new())
                .map_err(Into::into),
        }
    }
}

impl Set {
    pub fn apply(&self, buffer: bool) -> ApiResult {
        let Set(key, op, value) = self;
        let OptionInfos {
            commalist,
            flaglist,
            name,
            scope,
            ..
        } = do_on_error!(
            api::get_option_info(key),
            return Ok(()),
            error,
            "Invalid option {key}: {error}"
        );
        let set_option = set_option(scope, buffer)?;

        let get_option = get_option(scope, buffer)?;

        let current = SetValue::from_option(commalist, flaglist, &name, get_option(key)?)?;

        match (current, value.clone(), op) {
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
                log_error!("{op} {value:?} to {current:?} of {key} is not supported");
                return Ok(());
            }
        }
        .or_else(|err| {
            log_error!("Error while {op} {value:?} to {key}: \n{err}");
            Ok(())
        })
    }
}

fn set_option(
    scope: types::OptionScope,
    buffer: bool,
) -> ApiResult<fn(name: &str, value: SetValue) -> Result<()>> {
    Ok(match scope {
        types::OptionScope::Buffer if !buffer => |name, value| {
            api::set_option(name, value.clone())?;
            Buffer::current()
                .set_option(name, value)
                .map_err(Into::into)
        },
        types::OptionScope::Buffer => |name, value| {
            Buffer::current()
                .set_option(name, value)
                .map_err(Into::into)
        },
        types::OptionScope::Global if buffer => |name, value: SetValue| {
            api::set_option_value(
                name,
                value,
                &OptionValueOpts::builder().scope(OptionScope::Local).build(),
            )
            .map_err(Into::into)
        },
        types::OptionScope::Global => {
            |name, value: SetValue| api::set_option(name, value).map_err(Into::into)
        }
        types::OptionScope::Window => |name, value| {
            Window::current()
                .set_option(name, value)
                .map_err(Into::into)
        },
        _ => {
            return Err(ApiError::Other(format!(
                "Unsuported Option scope: {scope:?}"
            )))
        }
    })
}

fn get_option(
    scope: types::OptionScope,
    buffer: bool,
) -> ApiResult<fn(name: &str) -> ApiResult<Object>> {
    // dbg!((scope, buffer));
    Ok(match scope {
        types::OptionScope::Buffer => |name| Buffer::current().get_option(name),
        types::OptionScope::Global if buffer => |name| {
            api::get_option_value(
                name,
                &OptionValueOpts::builder().scope(OptionScope::Local).build(),
            )
        },
        types::OptionScope::Global => |name| {
            api::get_option_value(
                name,
                &OptionValueOpts::builder()
                    .scope(OptionScope::Global)
                    .build(),
            )
        },
        types::OptionScope::Window => {
            |name| -> ApiResult<Object> { Window::current().get_option(name) }
        }
        _ => {
            return Err(api::Error::Other(format!(
                "Unsuported Option scope: {scope:?}"
            )))
        }
    })
}
