//! Openskill bindings.

use std::{
    fmt::{self, Debug, Display, Formatter},
    process::Stdio,
    sync::Arc,
};

use chrono::TimeDelta;
use eyre::OptionExt as _;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::RwLock,
};

use crate::error::Error;

use super::{Matchup, Model, ModelData, Rating, RatingRecord};

pub type OpenSkillRating = Rating<OpenSkillData>;
pub type OpenSkillRatingRecord = RatingRecord<OpenSkillData>;

/// The openskill rating system.
#[derive(Clone)]
pub struct OpenSkill {
    config: Arc<OpenSkillConfig>,
    process: Arc<RwLock<Process>>,
}

impl OpenSkill {
    /// Creates a new `OpenSkill` interface.
    pub async fn new(config: OpenSkillConfig) -> eyre::Result<OpenSkill> {
        let command_parts = config
            .command
            .split(char::is_whitespace)
            .collect::<Vec<&str>>();

        // Start a process
        let mut child = Command::new(command_parts[0])
            .args(&command_parts[1..])
            .stderr(Stdio::inherit())
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()?;

        let mut process = Process {
            stdin: child.stdin.take().ok_or_eyre("no stdin exposed")?,
            stdout: child
                .stdout
                .take()
                .map(BufReader::new)
                .ok_or_eyre("no stdout exposed")?,
            _child: child,
        };

        // Send update config
        let _ = process
            .request(UpdateConfigRequest {
                config: config.clone(),
            })
            .await?;

        Ok(OpenSkill {
            config: Arc::new(config),
            process: Arc::new(RwLock::new(process)),
        })
    }
}

impl Model for OpenSkill {
    type Data = OpenSkillData;

    async fn create_rating(&self, user_id: i32) -> Result<Rating<Self::Data>, Error> {
        let mut process = self.process.write().await;

        let data = process.request(CreateRatingRequest { user_id }).await?;
        match data {
            Response::CreateRating(resp) => Ok(resp.rating),
            _ => Err(Error::new(UnexpectedResponse)),
        }
    }

    async fn rate(
        &self,
        rating: &RatingRecord<Self::Data>,
        matchups: &[Matchup<Self::Data>],
        _period_elapsed: f32,
    ) -> Result<Rating<Self::Data>, Error> {
        let mut process = self.process.write().await;

        let data = process
            .request(RateRequest {
                rating: rating.clone(),
                matchups: matchups.to_owned(),
            })
            .await?;
        match data {
            Response::Rate(resp) => Ok(resp.new_rating),
            _ => Err(Error::new(UnexpectedResponse)),
        }
    }

    async fn quality(&self, players: &[RatingRecord<Self::Data>]) -> Result<f32, Error> {
        let mut process = self.process.write().await;

        let data = process
            .request(QualityRequest {
                players: players.iter().cloned().map(Rating::from).collect(),
            })
            .await?;
        match data {
            Response::Quality(resp) => Ok(resp.quality),
            _ => Err(Error::new(UnexpectedResponse)),
        }
    }

    fn period(&self) -> chrono::TimeDelta {
        self.config.period
    }
}

impl Debug for OpenSkill {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenSkill")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

/// Does nothing but cache the ordinal.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenSkillData {
    ordinal: f32,
}

impl ModelData for OpenSkillData {
    fn ordinal(rating: &Rating<Self>) -> f32 {
        rating.extra.ordinal
    }
}

/// A request.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Request {
    UpdateConfig(UpdateConfigRequest),
    CreateRating(CreateRatingRequest),
    Quality(QualityRequest),
    Rate(RateRequest),
}

/// A response.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Response {
    UpdateConfig(UpdateConfigResponse),
    CreateRating(CreateRatingResponse),
    Quality(QualityResponse),
    Rate(RateResponse),
}

/// A request that initializes the rating system.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateConfigRequest {
    pub config: OpenSkillConfig,
}

impl From<UpdateConfigRequest> for Request {
    fn from(value: UpdateConfigRequest) -> Self {
        Request::UpdateConfig(value)
    }
}

/// A response for [`UpdateConfigResponse`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct UpdateConfigResponse {}

/// A request to [`Model::create_rating`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateRatingRequest {
    pub user_id: i32,
}

impl From<CreateRatingRequest> for Request {
    fn from(value: CreateRatingRequest) -> Self {
        Request::CreateRating(value)
    }
}

/// A response to [`Model::create_rating`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateRatingResponse {
    pub rating: OpenSkillRating,
}

/// A request to [`Model::quality`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QualityRequest {
    pub players: Vec<OpenSkillRating>,
}

impl From<QualityRequest> for Request {
    fn from(value: QualityRequest) -> Self {
        Request::Quality(value)
    }
}

/// A response to [`Model::quality`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct QualityResponse {
    pub quality: f32,
}

/// A request to [`Model::rate`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RateRequest {
    rating: OpenSkillRatingRecord,
    matchups: Vec<Matchup<OpenSkillData>>,
}

impl From<RateRequest> for Request {
    fn from(value: RateRequest) -> Self {
        Request::Rate(value)
    }
}

/// A response to [`Model::rate`].
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RateResponse {
    pub new_rating: OpenSkillRating,
}

#[derive(Clone, Copy, Debug)]
pub struct UnexpectedResponse;

impl Display for UnexpectedResponse {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("Unexpected response")
    }
}

impl std::error::Error for UnexpectedResponse {}

struct Process {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Process {
    /// Sends a request and gets a response back.
    pub async fn request<T>(&mut self, request: T) -> Result<Response, Error>
    where
        T: Into<Request>,
    {
        let request = request.into();

        // Serialize request
        let mut body = serde_json::to_string(&request).map_err(Error::new)?;
        body.push('\n');

        // Write body
        self.stdin
            .write_all(body.as_bytes())
            .await
            .map_err(Error::new)?;

        // Read result
        body.clear();
        self.stdout.read_line(&mut body).await.map_err(Error::new)?;

        // Deserialize
        serde_json::from_str::<Response>(body.trim()).map_err(Error::new)
    }
}

/// A config for `openskill`.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct OpenSkillConfig {
    /// The rating period.
    #[serde(
        deserialize_with = "crate::config::deserialize_duration",
        serialize_with = "crate::config::serialize_duration"
    )]
    pub period: TimeDelta,
    /// The command to start the open skill process.
    pub command: String,
    /// Prevents deviation from getting too small.
    pub tau: f32,
    /// Default settings for new players.
    pub defaults: InitialRating,
}

impl OpenSkillConfig {
    /// Builds a new `OpenSkill` model with the provided constants.
    ///
    /// This has to establish a connection to a process, so it is an
    /// asynchronous operation.
    pub async fn connect(self) -> eyre::Result<OpenSkill> {
        OpenSkill::new(self).await
    }
}

impl Default for OpenSkillConfig {
    fn default() -> Self {
        OpenSkillConfig {
            period: TimeDelta::seconds(86_400),
            command: "uv run main.py".into(),
            tau: 25.0 / 300.0,
            defaults: InitialRating::default(),
        }
    }
}

/// The initial rating of players.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct InitialRating {
    /// The rating new players start at.
    pub rating: f32,
    pub deviation: f32,
}

impl Default for InitialRating {
    fn default() -> Self {
        InitialRating {
            rating: 25.0,
            deviation: 25.0 / 3.0,
        }
    }
}
