use oxi::api::opts::{CreateAutocmdOpts, CreateAutocmdOptsBuilder};
use serde::Deserialize;
use serde_with::{serde_as, OneOrMany};

use crate::*;

#[serde_as]
#[derive(Debug, Deserialize, Default, PartialEq, Hash, Eq, Clone)]
#[serde(default)]
pub struct Condition {
    #[serde(default)]
    #[serde_as(deserialize_as = "OneOrMany<_>")]
    filetype: Vec<String>,
}

impl Condition {
    pub fn events(&self) -> Vec<String> {
        let mut ret = Vec::new();
        if !self.filetype.is_empty() {
            ret.push("FileType".to_string());
        }
        ret
    }
    pub fn opts(&self) -> CreateAutocmdOptsBuilder {
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
