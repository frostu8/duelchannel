//! User request types.

use duelchannel_model::{
    ApiError, Rrid, User,
    request::user::{CreateUser, CreateUserProfile},
};

use serde::Serialize;

use crate::{Client, Error};

/// A list players request.
#[derive(Debug, Serialize)]
pub struct ListPlayers {
    #[serde(skip)]
    client: Client,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_key: Option<Rrid>,
}

impl ListPlayers {
    /// Creates a new `ListPlayers` request.
    pub fn new(client: Client) -> ListPlayers {
        ListPlayers {
            client,
            count: None,
            public_key: None,
        }
    }

    /// The amount of players to return.
    pub fn count(self, count: i32) -> ListPlayers {
        ListPlayers {
            count: Some(count),
            ..self
        }
    }

    /// The public key to search with.
    pub fn public_key(self, public_key: Rrid) -> ListPlayers {
        ListPlayers {
            public_key: Some(public_key),
            ..self
        }
    }

    /// Fetches the list.
    pub async fn fetch(self) -> Result<Vec<User>, Error> {
        let url = format!("{}/players", self.client.state().endpoint);

        let res = self.client.client().get(url).query(&self).send().await?;
        if res.status().is_success() {
            res.json().await.map_err(Error::from)
        } else {
            Err(res.json::<ApiError>().await?.into())
        }
    }
}

/// A create player request.
#[derive(Debug)]
pub struct CreatePlayer {
    client: Client,
    inner: CreateUser,
}

impl CreatePlayer {
    /// Creates a new `CreatePlayer` request.
    pub fn new(client: Client, display_name: impl Into<String>) -> CreatePlayer {
        CreatePlayer {
            client,
            inner: CreateUser {
                display_name: display_name.into(),
                profiles: Vec::new(),
            },
        }
    }

    /// Adds a new profile.
    pub fn profile(mut self, public_key: Rrid) -> CreatePlayer {
        self.inner.profiles.push(CreateUserProfile { public_key });
        self
    }

    /// Executes the player creation.
    pub async fn execute(self) -> Result<User, Error> {
        let url = format!("{}/players", self.client.state().endpoint);

        let res = self
            .client
            .client()
            .post(url)
            .json(&self.inner)
            .send()
            .await?;
        if res.status().is_success() {
            res.json().await.map_err(Error::from)
        } else {
            Err(res.json::<ApiError>().await?.into())
        }
    }
}
