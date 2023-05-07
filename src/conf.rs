use config::Config;
use serde::{Serialize, Deserialize};
use std::path::PathBuf;
use crate::errors::BmailError;

#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    pub user: UserSettings,
    pub key: KeySettings,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UserSettings{
    pub handle: String,
    pub password: String
}

#[derive(Clone, Serialize, Deserialize)]
pub struct KeySettings{
    pub file_path: PathBuf,
}

/// Get configuration either from bmail.toml or from the env vars
pub fn get_configuration() -> Result<Settings, BmailError> {
    let base_path = std::env::current_dir().expect("Failed to determine the current directory");

    let settings = Config::builder()
        // Read the "default" configuration file
        .add_source(config::File::from(base_path.join("bmail")).required(true))
        // Add in settings from environment variables (with a prefix of APP and '__' as separator)
        // E.g. `APP_APPLICATION__PORT=5001 would set `Settings.application.port`
        .add_source(config::Environment::with_prefix("BMAIL").separator("_"))
        .build()?;

    settings.try_deserialize().map_err(Into::into)
}