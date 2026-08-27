//! Openskill bindings.

use std::{
    fmt::{self, Debug, Display, Formatter},
    num::NonZeroUsize,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

use chrono::TimeDelta;
use eyre::OptionExt as _;
use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot},
};

use crate::error::Error;

use super::{Matchup, Rating, RatingModel, RatingModelData};

pub type OpenSkillRating = Rating<OpenSkillData>;

/// The openskill rating system.
#[derive(Clone)]
pub struct OpenSkill {
    config: Arc<OpenSkillConfig>,
    workers: Arc<Vec<ProcessHandle>>,
    next_worker: Arc<AtomicUsize>,
}

impl OpenSkill {
    /// Creates a new `OpenSkill` interface.
    pub async fn new(config: OpenSkillConfig) -> eyre::Result<OpenSkill> {
        // Figure out how many cores to spawn.
        let worker_count = config
            .worker_count
            .map(|s| s.get())
            .unwrap_or_else(num_cpus::get);

        assert!(worker_count > 0);

        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let handle = ProcessHandle::spawn(&config.command)?;
            // Initialize worker with config
            let _ = handle
                .request(UpdateConfigRequest {
                    config: config.clone(),
                })
                .await?;

            workers.push(handle);
        }

        Ok(OpenSkill {
            config: Arc::new(config),
            workers: Arc::new(workers),
            next_worker: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Finds the next worker to use.
    fn next_worker(&self) -> &ProcessHandle {
        let idx = self.next_worker.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        &self.workers[idx]
    }
}

impl RatingModel for OpenSkill {
    type Data = OpenSkillData;

    async fn create_rating(&self, user_id: i32) -> Result<Rating<Self::Data>, Error> {
        let data = self
            .next_worker()
            .request(CreateRatingRequest { user_id })
            .await?;
        match data {
            Response::CreateRating(resp) => Ok(resp.rating),
            _ => Err(Error::new(UnexpectedResponse)),
        }
    }

    async fn rate(
        &self,
        rating: &Rating<Self::Data>,
        matchups: &[Matchup<Self::Data>],
        _period_elapsed: f32,
    ) -> Result<Rating<Self::Data>, Error> {
        let data = self
            .next_worker()
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

    async fn quality(&self, players: &[Rating<Self::Data>]) -> Result<f32, Error> {
        let data = self
            .next_worker()
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
    pub ordinal: f32,
}

impl RatingModelData for OpenSkillData {
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

impl Request {
    fn variant_name(&self) -> &'static str {
        match self {
            Request::UpdateConfig(_) => "UpdateConfig",
            Request::CreateRating(_) => "CreateRating",
            Request::Quality(_) => "Quality",
            Request::Rate(_) => "Rate",
        }
    }
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
    rating: OpenSkillRating,
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

/// An error returned when the open skill worker process has terminated.
#[derive(Clone, Copy, Debug)]
pub struct ProcessTerminated;

impl Display for ProcessTerminated {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("open skill worker process terminated")
    }
}

impl std::error::Error for ProcessTerminated {}

type Job = (Request, oneshot::Sender<Result<Response, Error>>);

#[derive(Clone)]
struct ProcessHandle {
    tx: mpsc::Sender<Job>,
}

impl ProcessHandle {
    /// Spawns a worker.
    fn spawn(command: &str) -> eyre::Result<ProcessHandle> {
        let command_parts = command.split(char::is_whitespace).collect::<Vec<&str>>();
        // Start a process
        let mut child = Command::new(command_parts[0])
            .args(&command_parts[1..])
            .stderr(Stdio::inherit())
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()?;
        let process = Process {
            stdin: child.stdin.take().ok_or_eyre("no stdin exposed")?,
            stdout: child
                .stdout
                .take()
                .map(BufReader::new)
                .ok_or_eyre("no stdout exposed")?,
            _child: child,
        };
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(run_process(process, rx));
        Ok(ProcessHandle { tx })
    }

    /// Sends a request to the worker and awaits its response.
    async fn request<T>(&self, request: T) -> Result<Response, Error>
    where
        T: Into<Request>,
    {
        let request = request.into();
        let kind = request.variant_name();

        // Time request
        let start = Instant::now();

        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send((request, reply_tx))
            .await
            .map_err(|_| Error::new(ProcessTerminated))?;

        let result = reply_rx.await.map_err(|_| Error::new(ProcessTerminated))?;

        tracing::debug!(
            request = kind,
            elapsed_ms = start.elapsed().as_secs_f64() * 1000.0,
            "openskill request completed"
        );
        result
    }
}

async fn run_process(mut process: Process, mut rx: mpsc::Receiver<Job>) {
    while let Some((request, reply)) = rx.recv().await {
        match process.request(request).await {
            Ok(response) => {
                let _ = reply.send(Ok(response));
            }
            Err(e) => {
                let _ = reply.send(Err(e));
                tracing::warn!("open skill worker failed, driver task exiting");
                return;
            }
        }
    }
}

struct Process {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Process {
    /// Sends a request and gets a response back.
    pub async fn request(&mut self, request: Request) -> Result<Response, Error> {
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
    /// The amount of workers to spawn.
    ///
    /// If unspecified, defaults to the number of cores.
    pub worker_count: Option<NonZeroUsize>,
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
            command: "uv run --package duelchannel-worker worker/main.py".into(),
            worker_count: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    use duelchannel_model::battle::BattleStatus;

    fn make_record(user_id: i32, rating: f32, deviation: f32) -> OpenSkillRating {
        OpenSkillRating {
            user_id,
            rating,
            deviation,
            extra: OpenSkillData {
                ordinal: rating - 3.0 * deviation,
            },
        }
    }

    #[tokio::test]
    async fn openskill_round_trip() {
        let model = OpenSkill::new(OpenSkillConfig::default())
            .await
            .expect("model to run");

        let rating = model.create_rating(1).await.expect("rating to be created");
        assert_eq!(rating.user_id, 1);
        assert!((rating.rating - 25.0).abs() < 1e-4, "mu default");
        assert!(
            (rating.deviation - 25.0 / 3.0).abs() < 1e-4,
            "sigma default"
        );

        let p1 = make_record(1, 25.0, 25.0 / 3.0);
        let p2 = make_record(2, 25.0, 25.0 / 3.0);
        let quality = model
            .quality(&[p1.clone(), p2.clone()])
            .await
            .expect("model rating quality");
        assert!(
            (quality - 1.0).abs() < 1e-3,
            "even match quality, got {quality}"
        );

        let matchup = Matchup {
            opponent: p2.clone(),
            status: BattleStatus::Concluded,
            position: 1,
            finish_time: 3000,
            no_contest: false,
        };
        let winner = model.rate(&p1, &[matchup], 0.0).await.expect("rating");
        assert!(
            winner.rating > p1.rating,
            "winner mu should increase: {} -> {}",
            p1.rating,
            winner.rating
        );
    }

    #[tokio::test]
    async fn openskill_concurrent_requests() {
        let model = OpenSkill::new(OpenSkillConfig::default())
            .await
            .expect("model to run");

        // Run many requests
        let mut handles = Vec::new();
        for i in 0..32 {
            let model = model.clone();
            handles.push(tokio::spawn(async move { model.create_rating(i).await }));
        }

        for (i, h) in handles.into_iter().enumerate() {
            let rating = h.await.unwrap().expect("rating to be created");
            assert_eq!(rating.user_id, i as i32);
        }
    }
}
