use oxi::api::{types::Mode, Buffer};
use serde::Deserialize;
use serde_with::{flattened_maybe, serde_as, OneOrMany};
use smart_default::SmartDefault;

use crate::*;

#[serde_as]
#[derive(Debug, Deserialize, SmartDefault, Clone)]
#[serde(default)]
pub struct Keys {
    #[serde_as(deserialize_as = "OneOrMany<_>")]
    modes: Vec<Mode>,
    #[default = true]
    recursive: bool,
    command: bool,
    lua: bool,
    silent: bool,
    unique: bool,
    expression: bool,
    leader: String,
    #[serde(flatten)]
    mappings_: HashMap<String, String>,
    mappings: HashMap<String, String>,
}
flattened_maybe!(deserialize_mappings, "mappings");

impl Keys {
    pub fn apply(&self, buffer: bool) -> ApiResult {
        for mode in &self.modes {
            for (lhs, rhs) in self.mappings.iter().chain(self.mappings_.iter()) {
                let cmd;
                let set_keymap: &fn(Mode, &str, &str, SetKeymapOpts) -> ApiResult<()> = &if buffer {
                    |a, b, c, d| Buffer::current().set_keymap(a, b, c, &d)
                } else {
                    |a, b, c, d| api::set_keymap(a, b, c, &d)
                };

                set_keymap(
                    *mode,
                    &(self.leader.clone() + lhs),
                    if self.lua {
                        cmd = format!(
                            "<CMD>lua {rhs}{}<CR>",
                            if rhs.ends_with(')') { "" } else { "()" }
                        );
                        &cmd
                    } else if self.command {
                        cmd = format!("<CMD>{rhs}<CR>");
                        &cmd
                    } else {
                        rhs
                    },
                    SetKeymapOpts::builder()
                        .noremap(!self.recursive)
                        .silent(self.silent)
                        .unique(self.unique)
                        .expr(self.expression)
                        .build(),
                )?;
            }
        }
        Ok(())
    }
}
