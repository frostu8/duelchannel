//! The actual client.

use std::{
    fmt::{self, Debug, Formatter},
    mem::take,
    sync::Arc,
};

use reqwest::header::{self, HeaderValue};

use crate::{
    Error,
    request::{
        battle::CreateBattle,
        user::{CreatePlayer, ListPlayers},
    },
};

/// The API client.
#[derive(Clone, Debug)]
pub struct Client {
    client: reqwest::Client,
    state: Arc<ClientState>,
}

impl Client {
    /// Creates a [`ClientBuilder`].
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// Creates a new battle.
    pub fn create_battle(&self, level_name: impl Into<String>) -> CreateBattle {
        CreateBattle::new(self.clone(), level_name)
    }

    /// Creates a new player.
    pub fn create_player(&self, display_name: impl Into<String>) -> CreatePlayer {
        CreatePlayer::new(self.clone(), display_name)
    }

    /// Fetches a list of players.
    pub fn list_players(&self) -> ListPlayers {
        ListPlayers::new(self.clone())
    }

    pub(crate) fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub(crate) fn state(&self) -> &ClientState {
        &self.state
    }
}

pub(crate) struct ClientState {
    pub endpoint: String,
}

impl Debug for ClientState {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientState")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// A builder for a [`Client`].
#[derive(Debug)]
pub struct ClientBuilder {
    inner: Result<ClientBuilderInner, Error>,
}

impl ClientBuilder {
    /// The endpoint to use for the client.
    pub fn endpoint(mut self, endpoint: impl Into<String>) -> ClientBuilder {
        match &mut self.inner {
            Ok(inner) => {
                *inner = ClientBuilderInner {
                    endpoint: endpoint.into(),
                    ..take(inner)
                }
            }
            Err(_) => (),
        }

        self
    }

    /// The key to use when authenticating requests.
    pub fn key(mut self, key: impl Into<String>) -> ClientBuilder {
        match &mut self.inner {
            Ok(inner) => {
                *inner = ClientBuilderInner {
                    key: Some(key.into()),
                    ..take(inner)
                }
            }
            Err(_) => (),
        }

        self
    }

    /// The user agent of the client.
    pub fn user_agent<V>(mut self, user_agent: V) -> ClientBuilder
    where
        V: TryInto<HeaderValue>,
        V::Error: Into<Error>,
    {
        let user_agent = match user_agent.try_into() {
            Ok(value) => value,
            Err(err) => {
                return ClientBuilder {
                    inner: Err(err.into()),
                };
            }
        };

        match &mut self.inner {
            Ok(inner) => {
                *inner = ClientBuilderInner {
                    user_agent: Some(user_agent),
                    ..take(inner)
                }
            }
            Err(_) => (),
        }

        self
    }

    /// Builds a client.
    pub fn build(self) -> Result<Client, Error> {
        match self.inner {
            Ok(inner) => {
                // Setup client
                let mut client = reqwest::Client::builder();
                if let Some(user_agent) = inner.user_agent {
                    client = client.user_agent(user_agent);
                }

                // Setup default headers
                let mut headers = header::HeaderMap::new();

                if let Some(key) = inner.key {
                    let mut value = header::HeaderValue::try_from(key)?;
                    value.set_sensitive(true);
                    headers.insert("X-API-KEY", value);
                }

                let client = client.default_headers(headers).build()?;

                Ok(Client {
                    client,
                    state: Arc::new(ClientState {
                        endpoint: inner.endpoint,
                    }),
                })
            }
            Err(err) => Err(err),
        }
    }
}

impl Default for ClientBuilder {
    fn default() -> ClientBuilder {
        ClientBuilder {
            inner: Ok(ClientBuilderInner::default()),
        }
    }
}

#[derive(Debug)]
struct ClientBuilderInner {
    endpoint: String,
    key: Option<String>,
    user_agent: Option<HeaderValue>,
}

impl Default for ClientBuilderInner {
    fn default() -> ClientBuilderInner {
        ClientBuilderInner {
            endpoint: String::from("https://duelchannel.ringrace.rs/api/v1"),
            key: None,
            user_agent: None,
        }
    }
}
