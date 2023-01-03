use std::{collections::HashMap, str::FromStr};

use anyhow::{ensure, Context, Error};
use derive_more::Display;
use macro_rules_attribute::derive;
use merge::Merge;
use nvim_oxi::mlua::lua;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DeserializeFromStr, OneOrMany, TryFromInto};

use crate::{ApiError, ApiResult};

type Result<T = (), E = Error> = std::result::Result<T, E>;

#[serde_as]
#[derive(Deserialize, Debug, Clone, Default)]
pub struct LuaConfig {
    #[serde_as(as = "HashMap<_, OneOrMany<_>>")]
    require: HashMap<Module, Vec<SubConfig>>,
}

impl Merge for LuaConfig {
    fn merge(&mut self, other: Self) {
        for (module, configs) in other.require {
            self.require
                .entry(module)
                .or_default()
                .extend_from_slice(&configs);
        }
    }
}

impl LuaConfig {
    pub fn apply(&self) -> ApiResult {
        let lua = lua();
        for (module, sub_configs) in &self.require {
            let module = module.as_str();
            for sub_config in sub_configs {
                let script = format!(r#"require({module:?}).{sub_config}"#);
                lua.load(&script)
                    .exec()
                    .map_err(|e| ApiError::Other(format!("{e}: {script:?}")))?;
            }
        }
        Ok(())
    }
}

type Module = String;

#[serde_as]
#[derive(Deserialize, Debug, Clone, Display)]
#[serde(untagged)]
pub enum SubConfig {
    FunctionCall(FunctionCall),
    Assignment(#[serde_as(as = "TryFromInto<HashMap<String, LuaValue>>")] Assignment),
    Expression(String),
}

#[serde_as]
#[derive(Deserialize, Debug, Clone, Display)]
#[serde(untagged)]
pub enum FunctionCall {
    #[display(fmt = "{_0}()")]
    Empty(FunctionName),
    WithArguments(
        #[serde_as(as = "TryFromInto<HashMap<FunctionName, LuaValue>>")] FunctionCallWithArguments,
    ),
}

#[serde_as]
#[derive(Deserialize, Serialize, Debug, Clone, Display)]
#[serde(untagged)]
pub enum LuaValue {
    Bool(bool),
    #[display(fmt = "{_0:?}")]
    String(String),
    Number(f64),
    #[display(
        fmt = "{{{}}}",
        "_0.iter().map(ToString::to_string).collect::<Vec<_>>().join(\",\")"
    )]
    List(Vec<LuaValue>),
    #[display(
        fmt = "{{{}}}",
        "_0.iter().map(|(key, value)| format!(\"[{key:?}]={value}\")).collect::<Vec<_>>().join(\",\")"
    )]
    Map(HashMap<String, LuaValue>),
}

#[derive(Debug, Clone, Display)]
// TODO implement a "structFnName" to support the difference between fn({a,b}) and fn(a,b)
#[display(fmt = "{function}({arguments})")]
pub struct FunctionCallWithArguments {
    pub function: FunctionName,
    pub arguments: LuaValue,
}

impl TryFrom<HashMap<FunctionName, LuaValue>> for FunctionCallWithArguments {
    type Error = Error;
    fn try_from(map: HashMap<FunctionName, LuaValue>) -> Result<Self> {
        ensure!(map.len() == 1);
        let (function, arguments) = map.into_iter().next().expect("map has len==1");

        Ok(Self {
            function,
            arguments,
        })
    }
}

#[derive(DeserializeFromStr, Debug, Clone, PartialEq, Eq, Hash, Display)]
pub struct FunctionName(String);

impl FromStr for FunctionName {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(
            s.strip_suffix("()")
                .context("FunctionName ends with ()")?
                .to_owned(),
        ))
    }
}

#[derive(Debug, Clone, Display)]
#[display(fmt = "{variable}={value}")]
pub struct Assignment {
    pub variable: String,
    pub value: LuaValue,
}

impl TryFrom<HashMap<String, LuaValue>> for Assignment {
    type Error = Error;
    fn try_from(map: HashMap<String, LuaValue>) -> Result<Self> {
        ensure!(map.len() == 1);
        let (variable, value) = map.into_iter().next().expect("map has len==1");

        Ok(Self { variable, value })
    }
}
