//! Application configuration.

use std::{
    path::{Path, PathBuf},
    str::FromStr,
};

use chrono::TimeDelta;

use duelchannel_model::user::UserFlags;
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
    value::Uncased,
};

use humantime::format_duration;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

use eyre::Error;

use crate::mmr::{glicko2::Glicko2Config, openskill::OpenSkillConfig};

/// Full application configuration.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Config {
    /// General server configuration.
    pub server: ServerConfig,
    /// Medal awards.
    pub awards: Vec<AwardConfig>,
    /// Mmr config.
    pub mmr: RatingModelConfig,
    /// Object storage configuration.
    pub cdn: StorageConfig,
    /// HTTP server configuration.
    pub http: HttpConfig,
    /// Discord configuration.
    pub discord: Option<DiscordConfig>,
}

/// General server configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServerConfig {
    /// The base url of the API.
    pub base_url: String,
    /// Where to send the client after they are done authenticating with the
    /// API.
    pub redirect_url: Option<String>,
    /// The database url to connect to.
    pub database_url: Option<String>,
    /// Whether to send session cookies (used for auth) with `Secure`.
    ///
    /// By default, this is `true` to avoid misconfiguration.
    pub secure_sessions: bool,
    /// Key used to encrypt cookies.
    pub encryption_key: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            base_url: "http://localhost:4000".into(),
            redirect_url: None,
            database_url: None,
            secure_sessions: true,
            encryption_key: None,
        }
    }
}

/// Configuration for awards.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AwardConfig {
    /// The ordinal threshold required to meet to receive the award.
    pub threshold: i32,
    /// If the award can be given while the user is in provisional ratings.
    pub award_provisional: bool,
    /// The awards to give out.
    #[serde(deserialize_with = "deserialize_awards_list")]
    pub awards: UserFlags,
}

impl Default for AwardConfig {
    fn default() -> Self {
        AwardConfig {
            threshold: 0,
            award_provisional: false,
            awards: UserFlags::empty(),
        }
    }
}

/// Configuration for object storage.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StorageConfig {
    /// The base url of the CDN.
    pub base_url: String,
    #[serde(flatten)]
    pub service: StorageService,
}

impl Default for StorageConfig {
    fn default() -> Self {
        StorageConfig {
            base_url: "http://localhost:4000".into(),
            service: StorageService::default(),
        }
    }
}

/// Configuration for object storage.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "service", rename_all = "snake_case")]
pub enum StorageService {
    Filesystem(FilesystemConfig),
    S3(S3Config),
}

impl Default for StorageService {
    fn default() -> Self {
        StorageService::Filesystem(FilesystemConfig::default())
    }
}

/// Filesystem object storage.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct FilesystemConfig {
    pub root: PathBuf,
}

impl Default for FilesystemConfig {
    fn default() -> Self {
        FilesystemConfig {
            root: PathBuf::from("./replays"),
        }
    }
}

/// Amazon S3 object storage.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct S3Config {
    /// The name of the bucket.
    pub bucket: String,
    /// The region of the S3.
    pub region: String,
    /// The access key ID.
    pub access_key_id: String,
    /// The secret access key.
    pub access_key_secret: String,
}

/// Configuration for MMR.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "model", rename_all = "snake_case")]
pub enum RatingModelConfig {
    Unrated,
    Glicko2(Glicko2Config),
    #[serde(rename = "openskill")]
    OpenSkill(OpenSkillConfig),
}

impl Default for RatingModelConfig {
    fn default() -> Self {
        RatingModelConfig::Unrated
    }
}

/// HTTP server configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HttpConfig {
    /// The port to listen on.
    pub port: u16,
}

impl Default for HttpConfig {
    fn default() -> Self {
        HttpConfig { port: 4000 }
    }
}

/// Discord OAuth2 configuration.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DiscordConfig {
    /// The client ID.
    pub client_id: u64,
    /// The client secret.
    pub client_secret: String,
}

/// Reads the configuration.
pub fn read_config(config_file: impl AsRef<Path>) -> Result<Config, Error> {
    Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::file(config_file))
        .merge(Env::prefixed("DUELCHANNEL_"))
        .merge(Env::raw().filter_map(|k| match k.as_str() {
            "DATABASE_URL" => Some(Uncased::from("server.database_url")),
            "DISCORD_CLIENT_ID" => Some(Uncased::from("discord.client_id")),
            "DISCORD_CLIENT_SECRET" => Some(Uncased::from("discord.client_secret")),
            "S3_ACCESS_KEY_ID" => Some(Uncased::from("cdn.access_key_id")),
            "S3_ACCESS_KEY_SECRET" => Some(Uncased::from("cdn.access_key_secret")),
            "ENCRYPTION_KEY" => Some(Uncased::from("server.encryption_key")),
            "PORT" => Some(Uncased::from("http.port")),
            _ => None,
        }))
        .extract()
        .map_err(From::from)
}

pub fn deserialize_duration<'de, D>(deserializer: D) -> Result<TimeDelta, D::Error>
where
    D: Deserializer<'de>,
{
    let text = String::deserialize(deserializer)?;
    let duration = humantime::parse_duration(&text).map_err(D::Error::custom)?;

    TimeDelta::from_std(duration).map_err(D::Error::custom)
}

pub fn serialize_duration<S>(delta: &TimeDelta, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    format_duration(delta.to_std().expect("positive time delta"))
        .to_string()
        .serialize(serializer)
}

fn deserialize_awards_list<'de, D>(deserializer: D) -> Result<UserFlags, D::Error>
where
    D: Deserializer<'de>,
{
    let awards = Vec::<String>::deserialize(deserializer)?;
    let awards = awards
        .iter()
        .map(|s| s.trim().parse::<UserFlags>())
        .collect::<Result<Vec<_>, <UserFlags as FromStr>::Err>>()
        .map_err(|err| D::Error::custom(err))?;

    Ok(awards
        .into_iter()
        .fold(UserFlags::empty(), |acc, x| acc | x))
}
