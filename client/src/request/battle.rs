//! Battle requests.

use duelchannel_model::{
    ApiError, Battle,
    request::battle::{CreateBattleParticipant, CreateBattleRequest},
};

use crate::{Client, Error};

/// A request to create a new match.
#[derive(Debug)]
pub struct CreateBattle {
    client: Client,
    inner: CreateBattleRequest,
}

impl CreateBattle {
    /// Creates a new `CreateMatch` request.
    pub fn new(client: Client, level_name: impl Into<String>) -> CreateBattle {
        CreateBattle {
            client,
            inner: CreateBattleRequest {
                level_name: level_name.into(),
                participants: vec![],
            },
        }
    }

    /// Adds a new participant to the match.
    pub fn participant(mut self, participant: impl Into<CreateBattleParticipant>) -> CreateBattle {
        self.inner.participants.push(participant.into());
        self
    }

    /// Creates the match.
    pub async fn execute(self) -> Result<Battle, Error> {
        let url = format!("{}/matches", self.client.state().endpoint);

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
