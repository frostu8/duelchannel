//! Replay MMR calculations for tuning.

use std::collections::HashMap;
use std::pin::pin;

use chrono::{DateTime, Utc};
use duelchannel_model::user::UserFlags;
use tokio::io::AsyncWriteExt;

use crate::mmr::{Matchup, Rating, RatingModel};

use duelchannel_model::battle::BattleStatus;

use sqlx::{FromRow, SqlitePool};

/// Options for running an MMR replay.
#[derive(Debug, Clone)]
pub struct ReplayOptions {
    /// Whether or not to print the header.
    pub print_header: bool,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        ReplayOptions { print_header: true }
    }
}

#[derive(Debug, FromRow)]
struct DuelResult {
    battle_id: i32,
    user_id: i32,
    finish_time: Option<i32>,
    no_contest: bool,
    concluded_at: DateTime<Utc>,
}

#[derive(Debug)]
struct Duel {
    id: i32,
    results: Vec<DuelResult>,
    concluded_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct UserHistory<T> {
    #[allow(dead_code)]
    id: i32,
    display_name: Option<String>,
    flags: UserFlags,
    rating: Rating<T>,
    last_update: Option<DateTime<Utc>>,
    wins: usize,
    losses: usize,
}

impl<T> UserHistory<T> {
    pub fn new(rating: Rating<T>) -> UserHistory<T> {
        UserHistory {
            id: rating.user_id,
            rating,
            // other fields
            display_name: None,
            flags: UserFlags::empty(),
            last_update: None,
            wins: 0,
            losses: 0,
        }
    }

    pub fn total_games(&self) -> usize {
        self.wins + self.losses
    }

    pub fn win_ratio(&self) -> f32 {
        if self.total_games() > 0 {
            self.wins as f32 / self.total_games() as f32
        } else {
            0.0
        }
    }
}

/// Replays MMR calculations for all players.
///
/// This only supports 1v1s as of writing.
pub async fn replay<T, W>(
    model: &T,
    output: W,
    db: &SqlitePool,
    options: ReplayOptions,
) -> eyre::Result<()>
where
    T: RatingModel,
    W: tokio::io::AsyncWrite,
{
    let mut output = pin!(output);

    // Initialize hashmaps
    let mut duels = HashMap::<i32, Duel>::new();
    let mut duel_order = Vec::<i32>::new();

    let mut user_history = HashMap::<i32, UserHistory<T::Data>>::new();

    // First, list ALL battles played on the local database, sorted by
    // conclusion order.
    let duel_results = sqlx::query_as::<_, DuelResult>(
        r#"
        SELECT
            b.id AS battle_id,
            b.concluded_at,
            p.user_id,
            p.finish_time,
            p.no_contest
        FROM battle b, participant p
        WHERE
            p.match_id = b.id
            AND b.status = 1
            AND b.concluded_at IS NOT NULL
        ORDER BY
            b.concluded_at ASC,
            b.id ASC
        "#,
    )
    .fetch_all(db)
    .await?;

    for duel_result in duel_results {
        let duel = duels.entry(duel_result.battle_id).or_insert_with(|| Duel {
            id: duel_result.battle_id,
            results: Vec::with_capacity(2),
            concluded_at: duel_result.concluded_at,
        });

        if duel.results.len() == 0 {
            // fresh duel, add it to order
            duel_order.push(duel.id);
        }

        duel.results.push(duel_result);
    }

    for id in duel_order {
        // Get duel from duels
        let duel = &duels[&id];
        let concluded_at = duel.concluded_at;

        let (p1, p2) = (&duel.results[0], &duel.results[1]);

        // Find the winner and loser
        let (winner, loser) = match (p1.no_contest, p2.no_contest) {
            (false, true) => (p1, p2),
            (true, false) => (p2, p1),
            // degenerate duel
            _ => continue,
        };

        for (me, opp, my_pos) in [(winner, loser, 1), (loser, winner, 2)] {
            let (me_rating, last_update) = match user_history.get(&me.user_id).cloned() {
                Some(UserHistory {
                    rating,
                    last_update,
                    ..
                }) => (rating, last_update),
                None => {
                    let rating = model.create_rating(me.user_id).await?;
                    let user = UserHistory::new(rating);

                    user_history.insert(me.user_id, user.clone());
                    (user.rating, user.last_update)
                }
            };
            let opp_rating = match user_history.get(&opp.user_id).cloned() {
                Some(UserHistory { rating, .. }) => rating,
                None => {
                    let rating = model.create_rating(opp.user_id).await?;
                    let user = UserHistory::new(rating);

                    user_history.insert(opp.user_id, user.clone());
                    user.rating
                }
            };

            // Get fractional period + grace period since the player's last
            // update
            let fractional_period = last_update
                .map(|t| {
                    ((concluded_at - t).as_seconds_f32() / model.period().as_seconds_f32())
                        - model.decay_grace()
                })
                .map(|t| t.clamp(0.0, 1.0))
                // player's first duel
                .unwrap_or(0.0);

            let matchup = Matchup {
                opponent: opp_rating,
                status: BattleStatus::Concluded,
                position: my_pos,
                finish_time: me.finish_time.unwrap_or(0),
                no_contest: me.no_contest,
            };
            let new_rating = model
                .rate(&me_rating, &[matchup], fractional_period)
                .await?;

            let user = user_history.get_mut(&me.user_id).unwrap();
            user.rating = new_rating;
            user.last_update = Some(concluded_at);

            if my_pos == 1 {
                user.wins += 1;
            } else {
                user.losses += 1;
            }
        }
    }

    // Get player display name
    let users =
        sqlx::query_as::<_, (i32, Option<String>, i32)>("SELECT id, display_name, flags FROM user")
            .fetch_all(db)
            .await?;
    for (id, display_name, flags) in users {
        let Some(uh) = user_history.get_mut(&id) else {
            continue;
        };

        uh.display_name = display_name;
        uh.flags = UserFlags::try_from(flags)?;
    }

    if options.print_header {
        output
            .write_all(format!("total_duels: {}\n", duels.len()).as_bytes())
            .await?;
        output
            .write_all(format!("total_users: {}\n", user_history.len()).as_bytes())
            .await?;
        output.write_all(b"\n").await?;
    }

    // Drop newlines and begin to print CSV, starting with the header
    let header = format!("id,name,games,wlr,rating,deviation,ordinal,st,challenger\n");
    output.write_all(header.as_bytes()).await?;

    // Print user data
    let mut uids = user_history.keys().copied().collect::<Vec<_>>();
    uids.sort_by_key(|u| {
        std::cmp::Reverse(user_history.get(u).map(|uh| uh.total_games()).unwrap_or(0))
    });

    for id in uids {
        let user = &user_history[&id];

        let provisional = if user.rating.is_provisional() {
            "PROV"
        } else {
            ""
        };
        let challenger = if user.flags.contains(UserFlags::BETA_CHALLENGER) {
            "YES"
        } else {
            "NO"
        };
        let data = format!(
            "{},{},{},{},{},{},{},{},{}\n",
            id,
            user.display_name
                .as_ref()
                .map_or("<empty>", |v| v)
                .replace(",", "\\,"),
            user.total_games(),
            user.win_ratio(),
            user.rating.rating,
            user.rating.deviation,
            user.rating.ordinal(),
            provisional,
            challenger,
        );
        output.write_all(data.as_bytes()).await?;
    }

    // Done with the replay
    Ok(())
}
