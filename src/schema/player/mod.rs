pub mod mmr;

use chrono::Utc;
use rand::{Rng, SeedableRng, distr::Alphanumeric};
use ring_channel_model::{Player, Rrid};
use sqlx::{FromRow, SqliteConnection};

use crate::{
    app::Model,
    error::{Error, ErrorKind},
};

use mmr::{Rating, RawRating};

const MAX_INSERT_ATTEMPTS: usize = 5;

/// A row in the database representing a player.
#[derive(FromRow)]
pub struct PlayerRow {
    #[sqlx(rename = "player_id")]
    pub id: i32,
    pub short_id: String,
    pub display_name: String,
    pub rating: Option<f32>,
    pub deviation: Option<f32>,
    #[sqlx(rename = "rating_extra")]
    pub extra: Option<String>,
}

impl PlayerRow {
    /// Converts a raw player into an API-ready player.
    pub fn normalize<T>(self, model: &Model<T>) -> Result<Player, Error>
    where
        T: mmr::Model + 'static,
    {
        let rating = if !model.ratings_enabled() {
            None
        } else if let Some((rating, deviation)) = self.rating.zip(self.deviation) {
            let rating = RawRating {
                player_id: self.id,
                rating,
                deviation,
                extra: self.extra,
            };

            Some(Rating::<T::Data>::try_from(rating).map_err(Error::new)?)
        } else {
            None
        };

        Ok(Player {
            id: self.short_id,
            display_name: self.display_name,
            mmr: rating.map(|rating| rating.ordinal() as i32),
            public_key: None,
        })
    }
}

/// Gets a player by their short id.
pub async fn get_player(
    short_id: &str,
    conn: &mut SqliteConnection,
) -> Result<Option<PlayerRow>, Error> {
    sqlx::query_as::<_, PlayerRow>(
        r#"
        SELECT
            id AS player_id,
            short_id,
            display_name,
            rating,
            deviation,
            rating_extra
        FROM
            player
        WHERE
            short_id = $1
        "#,
    )
    .bind(short_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(Error::from)
}

/// Inserts a player with a new short ID.
pub async fn create_player(
    public_key: &Rrid,
    display_name: &str,
    conn: &mut SqliteConnection,
) -> Result<PlayerRow, Error> {
    let mut rng = rand::rngs::StdRng::from_os_rng();
    create_player_with(public_key, display_name, conn, &mut rng).await
}

/// Inserts a player with a new short ID.
pub async fn create_player_with<R>(
    public_key: &Rrid,
    display_name: &str,
    conn: &mut SqliteConnection,
    rng: &mut R,
) -> Result<PlayerRow, Error>
where
    R: Rng,
{
    let now = Utc::now();

    // this is a new player
    let mut inserted_player = None::<PlayerRow>;

    for _ in 0..MAX_INSERT_ATTEMPTS {
        // generate a short id
        let short_id = rng
            .sample_iter(Alphanumeric)
            .take(6)
            .map(char::from)
            .map(|c| char::to_ascii_uppercase(&c))
            .collect::<String>();

        // try to insert with short_id
        let result = sqlx::query_as::<_, PlayerRow>(
            r#"
            INSERT INTO player
                (
                    short_id,
                    public_key,
                    display_name,
                    inserted_at,
                    updated_at
                )
            VALUES ($1, $2, $3, $4, $4)
            RETURNING id AS player_id, short_id, display_name, rating, deviation, rating_extra
            "#,
        )
        .bind(&short_id)
        .bind(public_key.as_str())
        .bind(display_name)
        .bind(now)
        .fetch_one(&mut *conn)
        .await;

        match result {
            Ok(player) => {
                inserted_player = Some(player);
                break;
            }
            Err(err) => {
                if let Some(db_err) = err.as_database_error() {
                    // if this is a unique violation, simply try again
                    if db_err.is_unique_violation() {
                        tracing::debug!("unique key {} failed, regenerating", short_id);
                    } else {
                        return Err(err.into());
                    }
                } else {
                    return Err(err.into());
                }
            }
        }
    }

    inserted_player.ok_or_else(|| ErrorKind::OutOfIds.into())
}
