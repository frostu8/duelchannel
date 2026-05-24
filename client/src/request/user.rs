//! User request types.

use duelchannel_model::{
    ApiError, Rrid, User,
    request::user::{CreateUser, CreateUserProfile, ListUsers},
};

use crate::{Client, Error};

/// A list players request.
#[derive(Debug)]
pub struct ListPlayers {
    client: Client,
    inner: ListUsers,
}

impl ListPlayers {
    /// Creates a new `ListPlayers` request.
    pub fn new(client: Client) -> ListPlayers {
        ListPlayers {
            client,
            inner: ListUsers::default(),
        }
    }

    /// The amount of players to return.
    pub fn count(mut self, count: i32) -> ListPlayers {
        self.inner.count = Some(count);
        self
    }

    /// The public key to search with.
    pub fn public_key(mut self, public_key: Rrid) -> ListPlayers {
        self.inner.public_key = Some(public_key);
        self
    }

    /// Fetches the list.
    pub async fn fetch(self) -> Result<Vec<User>, Error> {
        let url = format!("{}/players", self.client.state().endpoint);

        let res = self
            .client
            .client()
            .get(url)
            .query(&self.inner)
            .send()
            .await?;
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
