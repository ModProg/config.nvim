use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

use crate::*;

fn hashes_file() -> PathBuf {
    let stdpath: String =
        api::call_function("stdpath", ("data",)).expect("There is a stdpath for data");
    PathBuf::from(stdpath).join("config/hashes")
}

#[derive(Deserialize, Serialize, Debug, Default, Clone)]
pub struct Hashes(HashMap<PathBuf, Vec<u8>>);
impl Hashes {
    pub fn is_hashed(&self, path: &Path, config: &str) -> bool {
        if let Some(hash) = self.0.get(path) {
            let mut hasher = Sha512::new();
            hasher.update(config);
            let result = hasher.finalize();
            hash == result.as_slice()
        } else {
            false
        }
    }
    pub fn add_hash(&mut self, path: PathBuf, config: &str) {
        self.0.insert(path, {
            let mut hasher = Sha512::new();
            hasher.update(config);
            hasher.finalize().to_vec()
        });
    }

    pub fn load() -> Option<Self> {
        rmp_serde::from_slice(&fs::read(hashes_file()).ok()?).ok()?
    }

    pub fn unhashed(
        &self,
        files: impl IntoIterator<Item = (PathBuf, String, Config)>,
    ) -> (Vec<PathBuf>, Vec<Config>) {
        files.into_iter().partition_map(|(path, string, config)| {
            if self.is_hashed(&path, &string) {
                Either::Right(config)
            } else {
                Either::Left(path)
            }
        })
    }

    pub fn save(&self) -> ApiResult<()> {
        let hashes_file = hashes_file();
        let data_dir = hashes_file.parent().expect("Hashes file has a parent");
        fs::create_dir_all(data_dir).map_err(|e| {
            api::Error::Other(format!(
                "Error while creating data dir `{}`: {e}",
                data_dir.display()
            ))
        })?;
        fs::write(
            &hashes_file,
            rmp_serde::to_vec(&self).expect("Hashes serialization is infallible"),
        )
        .map_err(|e| {
            api::Error::Other(format!(
                "Error while saving hashes `{}`: {e}",
                hashes_file.display()
            ))
        })?;
        Ok(())
    }
}
