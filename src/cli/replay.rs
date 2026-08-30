//! Replay MMR calculations for tuning.

use std::cmp::min;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{self, Display, Formatter};
use std::pin::pin;

use chrono::{DateTime, TimeDelta, Utc};
use duelchannel_model::user::UserFlags;
use sea_query::{Expr, ExprTrait, Iden, JoinType, Order, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use tokio::io::AsyncWriteExt;

use crate::config::Config;
use crate::mmr::{Matchup, Rating, RatingModel};

use duelchannel_model::battle::BattleStatus;

use sqlx::{FromRow, SqlitePool};

/// Options for running an MMR replay.
#[derive(Debug, Clone)]
pub struct ReplayOptions {
    /// Only replay for these players (by short id).
    pub players: Option<HashSet<String>>,
    /// Whether or not to print the header.
    pub print_header: bool,
    pub replay_since: Option<DateTime<Utc>>,
    /// Replay up to a certain date-time.
    ///
    /// If `None`, this will replay up to the last battle.
    pub replay_to: Option<DateTime<Utc>>,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        ReplayOptions {
            players: None,
            print_header: true,
            replay_since: None,
            replay_to: None,
        }
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
struct RatingPeriod<T> {
    started_at: DateTime<Utc>,
    players: HashMap<i32, Rating<T>>,
    duels: Vec<i32>,
}

impl<T> RatingPeriod<T> {
    pub fn new(started_at: DateTime<Utc>) -> RatingPeriod<T> {
        RatingPeriod {
            started_at,
            players: HashMap::new(),
            duels: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct UserHistory<T> {
    #[allow(dead_code)]
    id: i32,
    short_id: String,
    display_name: Option<String>,
    flags: UserFlags,
    rating: Rating<T>,
    periods_played: BTreeSet<usize>,
    wins: usize,
    losses: usize,
}

impl<T> UserHistory<T> {
    pub fn new(rating: Rating<T>) -> UserHistory<T> {
        UserHistory {
            id: rating.user_id,
            rating,
            // other fields
            short_id: String::new(),
            display_name: None,
            flags: UserFlags::empty(),
            periods_played: BTreeSet::new(),
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

#[derive(Debug)]
struct ReplayEngine<T>
where
    T: RatingModel,
{
    pub replay_to: DateTime<Utc>,
    pub duels: HashMap<i32, Duel>,
    pub user_history: HashMap<i32, UserHistory<T::Data>>,
    pub rating_periods: Vec<RatingPeriod<T::Data>>,
}

impl<T> ReplayEngine<T>
where
    T: RatingModel,
{
    pub fn new(replay_to: DateTime<Utc>) -> Self {
        ReplayEngine {
            user_history: HashMap::new(),
            duels: HashMap::new(),
            rating_periods: Vec::new(),
            replay_to,
        }
    }

    async fn rate(
        &mut self,
        user_id: i32,
        period_index: usize,
        matchups: &[Matchup<T::Data>],
        model: &T,
    ) -> eyre::Result<Rating<T::Data>> {
        let period = &self.rating_periods[period_index];
        let ended_at = period.started_at + model.period();

        let updating_at = min(ended_at, self.replay_to);

        let me_rating = match period.players.get(&user_id) {
            Some(r) => r.clone(),
            None => {
                // Player didn't play last period. Either the player's rating
                // failed to rollover, or they're just new.
                model.create_rating(user_id).await?
            }
        };

        let user = self
            .user_history
            .entry(user_id)
            .or_insert_with(|| UserHistory::new(me_rating.clone()));

        // Get how many periods the player has been idle
        let idle_time = user
            .periods_played
            .iter()
            .copied()
            .rfind(|&idx| idx <= period_index)
            .map(|idx| self.rating_periods[idx].started_at)
            .map(|t| updating_at - t);
        if let Some(idle_time) = idle_time {
            assert!(idle_time > TimeDelta::zero());
        }

        // Get fractional period + grace period since the player's last
        // update
        let fractional_period = idle_time
            .map(|t| (t.as_seconds_f32() / model.period().as_seconds_f32()) - model.decay_grace())
            .map(|t| t.clamp(0.0, 1.0))
            // player's first duel
            .unwrap_or(0.0);

        let new_rating = model.rate(&me_rating, matchups, fractional_period).await?;

        user.rating = new_rating;
        user.wins += matchups
            .iter()
            .filter_map(|mu| bool::then_some(mu.position == 1, 1))
            .sum::<usize>();
        user.losses += matchups
            .iter()
            .filter_map(|mu| bool::then_some(mu.position != 1, 1))
            .sum::<usize>();

        // Register player for next period
        if let Some(next_period) = self.rating_periods.get_mut(period_index + 1) {
            next_period.players.insert(user_id, user.rating.clone());
        }

        Ok(user.rating.clone())
    }

    async fn rate_period(&mut self, period_index: usize, model: &T) -> eyre::Result<()> {
        let period = &mut self.rating_periods[period_index];

        // Iterate over duels
        let mut matchups = HashMap::<i32, Vec<Matchup<T::Data>>>::new();
        for id in period.duels.iter() {
            // Get duel from duels
            let duel = &self.duels[&id];
            let (p1, p2) = (&duel.results[0], &duel.results[1]);

            // Find the winner and loser
            let (winner, loser) = match (p1.no_contest, p2.no_contest) {
                (false, true) => (p1, p2),
                (true, false) => (p2, p1),
                // degenerate duel
                _ => continue,
            };

            for (me, opp, my_pos) in [(winner, loser, 1), (loser, winner, 2)] {
                let opp_rating = match period.players.get(&opp.user_id) {
                    Some(r) => r.clone(),
                    None => {
                        let rating = model.create_rating(opp.user_id).await?;
                        period.players.insert(opp.user_id, rating.clone());

                        rating
                    }
                };

                let matchups = matchups.entry(me.user_id).or_default();
                matchups.push(Matchup {
                    opponent: opp_rating,
                    status: BattleStatus::Concluded,
                    position: my_pos,
                    finish_time: me.finish_time.unwrap_or_default(),
                    no_contest: me.no_contest,
                });
            }
        }

        // Rate players per matchups
        let user_ids = self
            .user_history
            .values()
            .map(|uh| uh.id)
            .collect::<Vec<_>>();
        for user_id in user_ids {
            let matchups = matchups.get(&user_id).map(|v| v.as_slice()).unwrap_or(&[]);
            self.rate(user_id, period_index, matchups, model).await?;
        }

        Ok(())
    }
}

#[derive(Iden)]
enum Table {
    Battle,
    Participant,
}

/// Replays MMR calculations for all players.
///
/// This only supports 1v1s as of writing.
pub async fn replay<T, W>(
    model: &T,
    output: W,
    db: &SqlitePool,
    config: &Config,
    options: ReplayOptions,
) -> eyre::Result<()>
where
    T: RatingModel,
    W: tokio::io::AsyncWrite,
{
    let mut output = pin!(output);

    // First, list ALL battles played on the local database, sorted by
    // conclusion order.
    let (query, values) = Query::select()
        .expr_as(Expr::col((Table::Battle, "id")), "battle_id")
        .column((Table::Battle, "concluded_at"))
        .column((Table::Participant, "user_id"))
        .column((Table::Participant, "finish_time"))
        .column((Table::Participant, "no_contest"))
        .from(Table::Battle)
        .join(
            JoinType::Join,
            Table::Participant,
            Expr::col((Table::Battle, "id")).equals((Table::Participant, "match_id")),
        )
        .and_where(Expr::col((Table::Battle, "status")).eq(1))
        .and_where(Expr::col((Table::Battle, "concluded_at")).is_not_null())
        .order_by((Table::Battle, "concluded_at"), Order::Asc)
        .order_by((Table::Battle, "id"), Order::Asc)
        .apply_if(options.replay_since, |q, since| {
            q.and_where(Expr::col((Table::Battle, "concluded_at")).gte(since));
        })
        .build_sqlx(SqliteQueryBuilder);

    let duel_results = sqlx::query_as_with::<_, DuelResult, _>(sqlx::AssertSqlSafe(query), values)
        .fetch_all(db)
        .await?;

    assert!(duel_results.len() > 0, "need at least one duel to replay");

    let replay_to = options
        .replay_to
        .or_else(|| {
            duel_results
                .iter()
                .map(|duel_result| duel_result.concluded_at)
                .max()
        })
        .unwrap();

    // Get player info
    let db_users = sqlx::query_as::<_, (i32, String, Option<String>, i32)>(
        "SELECT id, short_id, display_name, flags FROM user",
    )
    .fetch_all(db)
    .await?;

    // Initialize hashmaps
    let mut replay_engine = ReplayEngine::<T>::new(replay_to);

    let mut user_order = Vec::<i32>::new();
    let mut include_players = HashSet::<i32>::new();

    for (user_id, short_id, display_name, flags) in db_users {
        let uh = match replay_engine.user_history.get_mut(&user_id) {
            Some(uh) => uh,
            None => {
                // Initialize rating
                let rating = model.create_rating(user_id).await?;
                replay_engine
                    .user_history
                    .insert(user_id, UserHistory::new(rating));
                replay_engine.user_history.get_mut(&user_id).unwrap()
            }
        };

        uh.short_id = short_id.clone();
        uh.display_name = display_name;
        uh.flags = UserFlags::try_from(flags)?;

        // Check if we need to include this player
        if let Some(players) = options.players.as_ref() {
            if players.contains(&short_id) {
                include_players.insert(user_id);
            }
        } else {
            // Always include player
            include_players.insert(user_id);
        }

        user_order.push(user_id);
    }

    let start_at = options
        .replay_since
        .or_else(|| duel_results.iter().next().map(|d| d.concluded_at))
        .expect("at least one duel");

    for duel_result in duel_results {
        let duel = replay_engine
            .duels
            .entry(duel_result.battle_id)
            .or_insert_with(|| Duel {
                id: duel_result.battle_id,
                results: Vec::with_capacity(2),
                concluded_at: duel_result.concluded_at,
            });

        // Initialize rating periods
        let (period_idx, period) =
            if let Some((idx, p)) = replay_engine.rating_periods.iter_mut().enumerate().last() {
                let ended_at = p.started_at + model.period();
                if duel.concluded_at > ended_at {
                    // Create new rating period
                    let idx = replay_engine.rating_periods.len();

                    replay_engine
                        .rating_periods
                        .push(RatingPeriod::new(ended_at));
                    (idx, &mut replay_engine.rating_periods[idx])
                } else {
                    (idx, p)
                }
            } else {
                let idx = replay_engine.rating_periods.len();

                replay_engine
                    .rating_periods
                    .push(RatingPeriod::new(start_at));
                (idx, &mut replay_engine.rating_periods[idx])
            };

        if duel.results.len() == 0 {
            // fresh duel, add it to order
            period.duels.push(duel.id);
        }

        if let Some(uh) = replay_engine.user_history.get_mut(&duel_result.user_id) {
            uh.periods_played.insert(period_idx);
        }

        duel.results.push(duel_result);
    }

    // Pad extra rating periods
    let mut started_at = replay_engine
        .rating_periods
        .iter()
        .last()
        .map(|rp| rp.started_at)
        .expect("at least one rating period");
    while started_at < replay_to {
        // Add rating period for posterity
        started_at += model.period();
        replay_engine
            .rating_periods
            .push(RatingPeriod::new(started_at));
    }

    for i in 0..replay_engine.rating_periods.len() {
        replay_engine.rate_period(i, model).await?;
    }

    let ReplayEngine {
        user_history,
        duels,
        ..
    } = replay_engine;

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
    let header = format!(
        "id,short_id,name,games,wlr,rating,deviation,ordinal,provisional,medal,new_medal\n"
    );
    output.write_all(header.as_bytes()).await?;

    // Print user data
    let mut uids = user_history.keys().copied().collect::<Vec<_>>();
    uids.sort_by_key(|u| {
        std::cmp::Reverse(user_history.get(u).map(|uh| uh.total_games()).unwrap_or(0))
    });

    for id in uids {
        if !include_players.contains(&id) {
            continue;
        }

        let user = &user_history[&id];

        let provisional = if user.rating.is_provisional() {
            "PROV"
        } else {
            "VISIBLE"
        };

        let historical_challenger = user.flags.contains(UserFlags::BETA_CHALLENGER);
        let potential_challenger = config
            .awards
            .values()
            .filter(|award| award.threshold as f32 <= user.rating.ordinal())
            .filter(|award| !user.rating.is_provisional() || award.award_provisional)
            .any(|award| award.flag.contains(UserFlags::BETA_CHALLENGER));

        let data = format!(
            "{},{},{},{},{},{},{},{},{},{},{}\n",
            id,
            user.short_id,
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
            YesNo(historical_challenger),
            YesNo(potential_challenger),
        );
        output.write_all(data.as_bytes()).await?;
    }

    // Done with the replay
    Ok(())
}

#[derive(Debug)]
struct YesNo(bool);

impl Display for YesNo {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        if self.0 {
            f.write_str("YES")
        } else {
            f.write_str("NO")
        }
    }
}
