#![warn(clippy::pedantic)]
#![allow(
    clippy::struct_excessive_bools,
    clippy::too_many_lines,
    clippy::unnecessary_wraps,
    clippy::wildcard_imports,
    clippy::module_name_repetitions
)]
#![warn(clippy::unwrap_used)]
use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
};

use itertools::{Either, Itertools};
pub use nvim_oxi as oxi;
pub use oxi::{api, api::Error as ApiError, Error};
use oxi::{
    api::{opts::*, types::LogLevel},
    conversion, Dictionary, Function,
};
use walkdir::WalkDir;

#[macro_use]
mod macros;

mod config;
use config::*;

mod hashes;
use hashes::*;

type Result<T = (), E = oxi::Error> = std::result::Result<T, E>;
type ApiResult<T = ()> = Result<T, ApiError>;
type ConvResult<T = ()> = Result<T, conversion::Error>;

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

fn get_config_dirs() -> Vec<PathBuf> {
    let mut nvim_folders = Vec::new();
    let Ok(cwd) = env::current_dir() else { return Vec::new() };
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
    let get_files = |pattern: &'static str| {
        // api::call_function("globpath", (rtp, pattern))
        api::get_runtime_file(pattern, true)
    };

    let mut conditional_configs: HashMap<Condition, Config> = HashMap::new();

    for path in get_files("config/*.toml")?
        .chain(get_files("config/*.yaml")?)
        .chain(get_files("config/*.json")?)
        .chain(get_files("config/*.toml")?)
    {
        continue_on_error!(Config::load(path.as_path()), error, "{error}")
            .0
            .merge_into_hashmap(&mut conditional_configs);
    }

    let mut hashes = Hashes::load().unwrap_or_default();

    let config_files: Vec<_> = get_config_dirs()
        .iter()
        .flat_map(|path| config_files(path))
        .collect();
    let (unknown, known) = hashes.unhashed(config_files);
    for config in known {
        config.merge_into_hashmap(&mut conditional_configs);
    }
    if !unknown.is_empty() {
        {
            let unknown: Vec<_> = unknown.iter().map(|p| p.to_string_lossy()).collect();
            api::notify(
                &format!(
                    "Found new local config{}: \n  {}\nRun :ConfigAllow to activate",
                    (unknown.len() > 1).then_some("s").unwrap_or_default(),
                    unknown.join("\n  ")
                ),
                LogLevel::Info,
                &NotifyOpts::default(),
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
                hashes.save()?;
                Ok(())
            },
            &CreateCommandOpts::default(),
        )?;
    }

    if let Some(config) = conditional_configs.remove(&Condition::default()) {
        config.apply(false)?;
    }
    // for (condition, config) in conditional_configs {
    //     api::create_autocmd(
    //         condition.events().iter().map(AsRef::as_ref),
    //         &condition
    //             .opts()
    //             .callback(move |_| -> Result<bool> {
    //                 config.apply(true)?;
    //                 Ok(false)
    //             })
    //             .build(),
    //     )
    //     .expect("Create autocommand for conditional config");
    // }
    Ok(())
}

#[oxi::module]
fn config() -> Result<Dictionary, nvim_oxi::Error> {
    Ok(Dictionary::from_iter([(
        "load_config",
        Function::from_fn(load_config),
    )]))
}
