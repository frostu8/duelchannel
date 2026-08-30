//! A [Glicko-2][1] implementation with modifications to allow instant results.
//!
//! [1]: https://www.glicko.net/glicko/glicko2.pdf

use std::{f32::consts::PI, sync::Arc};

use chrono::TimeDelta;
use serde::{Deserialize, Serialize};

use crate::error::Error;

use super::{Rating, RatingModel, RatingModelData};

pub const CONVERGENCE_TOLERANCE: f32 = 0.000_001;

/// The Glicko-2 model.
#[derive(Clone, Debug)]
pub struct Glicko2 {
    config: Arc<Glicko2Config>,
}

impl Glicko2 {
    pub fn new(config: Glicko2Config) -> Glicko2 {
        config.into()
    }
}

impl From<Glicko2Config> for Glicko2 {
    fn from(value: Glicko2Config) -> Self {
        Glicko2 {
            config: Arc::new(value),
        }
    }
}

impl RatingModel for Glicko2 {
    type Data = Glicko2Data;

    async fn create_rating(&self, player_id: i32) -> Result<Rating<Self::Data>, Error> {
        Ok(Rating {
            user_id: player_id,
            rating: self.config.defaults.rating,
            deviation: self.config.defaults.deviation,
            extra: Glicko2Data {
                volatility: self.config.defaults.volatility,
            },
        })
    }

    async fn rate(
        &self,
        rating: &Rating<Self::Data>,
        matchups: &[super::Matchup<Self::Data>],
        period_elapsed: f32,
    ) -> Result<Rating<Self::Data>, Error> {
        let matchups = matchups
            .iter()
            .map(|matchup| Matchup {
                opponent: matchup.opponent.clone(),
                outcome: if matchup.position > 1 {
                    Outcome::Lose
                } else {
                    Outcome::Win
                },
            })
            .collect::<Vec<_>>();

        Ok(rate(&self.config, rating, &matchups, period_elapsed))
    }

    async fn quality(&self, players: &[Rating<Self::Data>]) -> Result<f32, Error> {
        assert!(players.len() == 2);
        let a = &players[0];
        let b = &players[1];

        let (mu_a, phi_a) = to_glicko2(a);
        let (mu_b, phi_b) = to_glicko2(b);

        // Probability A beats B
        let p_ab = e_func(mu_a, mu_b, g_func(phi_b));
        // Probability B beats A
        let p_ba = e_func(mu_b, mu_a, g_func(phi_a));

        let quality = 4.0 * (p_ab * (1.0 - p_ab) * p_ba * (1.0 - p_ba)).sqrt();
        Ok(quality)
    }

    fn period(&self) -> TimeDelta {
        self.config.period
    }

    fn decay_grace(&self) -> f32 {
        self.config.decay_grace.as_seconds_f32() / self.config.period.as_seconds_f32()
    }
}

/// Contains the "volatility" of Glicko2 ratings.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Glicko2Data {
    pub volatility: f32,
}

const TOP: f32 = 25000.0;
const MID: f32 = 1500.0;
const SLOPE: f32 = 0.117; // regressed from tetr.io 2023 rating data

impl RatingModelData for Glicko2Data {
    /// Displayed rating.
    ///
    /// This is based off tetr.io's "TR" and uses this page on GLIXARE as a
    /// basis.
    ///
    /// Read more:
    /// <https://www.smogon.com/forums/threads/gxe-glixare-a-much-better-way-of-estimating-a-players-overall-rating-than-shoddys-cre.51169/>
    fn ordinal(rating: &Rating<Self>) -> f32 {
        let deviation = rating.deviation.max(1e-3);
        TOP / (1.0 + 10f32.powf((MID - rating.rating) * SLOPE / deviation))
    }

    fn is_provisional(rating: &Rating<Self>) -> bool {
        // Only show ratings that are <100 RD
        rating.deviation > 100.0
    }
}

pub type Glicko2Rating = Rating<Glicko2Data>;

/// Configuration for MMR.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Glicko2Config {
    /// The rating period.
    ///
    /// This should be set to a reasonable value for a single player to get at
    /// least 10 matches in, but it shouldn't be too high.
    #[serde(
        deserialize_with = "crate::config::deserialize_duration",
        serialize_with = "crate::config::serialize_duration"
    )]
    pub period: TimeDelta,
    /// Constrains the change in volatility over time.
    ///
    /// Higher values may make skill volatility change more frequently, and
    /// lower values make it stay around the same.
    ///
    /// See the [Glicko-2] paper for more.
    ///
    /// [Glicko-2]: https://www.glicko.net/glicko/glicko2.pdf
    pub tau: f32,
    /// A soft lower limit on rating deviation.
    pub min_deviation: f32,
    /// How long a player can be inactive before their rating deviation decays.
    #[serde(
        deserialize_with = "crate::config::deserialize_duration",
        serialize_with = "crate::config::serialize_duration"
    )]
    pub decay_grace: TimeDelta,
    /// Default settings for new players.
    pub defaults: InitialRating,
}

impl Default for Glicko2Config {
    fn default() -> Self {
        Glicko2Config {
            period: TimeDelta::seconds(86_400),
            tau: 0.5,
            min_deviation: 60.0,
            decay_grace: TimeDelta::weeks(1),
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
    pub volatility: f32,
}

impl Default for InitialRating {
    fn default() -> Self {
        InitialRating {
            rating: 1500.0,
            deviation: 350.0,
            volatility: 0.06,
        }
    }
}

#[derive(Clone, Debug)]
struct Matchup {
    /// The opponent player's rating at the start of the period
    pub opponent: Glicko2Rating,
    /// The outcome of the match, in the perspective of the player, *not* the
    /// opponent.
    pub outcome: Outcome,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Outcome {
    Win,
    Lose,
}

/// Rates a player's performance.
///
/// Returns a new player rating.
fn rate(
    config: &Glicko2Config,
    player: &Glicko2Rating,
    matches: &[Matchup],
    fractional_period: f32,
) -> Rating<Glicko2Data> {
    assert!((0f32..=1f32).contains(&fractional_period));

    // Step 1 has already been done for us in the database.

    // Step 2: Convert into Glicko-2 scale.
    let (mu, phi) = to_glicko2(player);

    if matches.len() == 0 {
        // If the player didn't play any matches, only Step 6 applies.
        let new_phi = calculate_pre_rating_period_value(player.volatility, phi, fractional_period);

        return Rating {
            deviation: new_phi * 173.7178,
            ..Rating::<Glicko2Data>::from(player.clone())
        };
    }

    // Step 3: Estimate the variance of player's rating based on the game
    // outcomes.
    let v = matches
        .iter()
        .map(|matchup| {
            // Calculate opponent glicko2 stats
            let (opponent_mu, opponent_phi) = to_glicko2(&matchup.opponent);

            let g = g_func(opponent_phi);
            let e = e_func(mu, opponent_mu, g);

            g * g * e * (1.0 - e)
        })
        .sum::<f32>()
        .recip();

    // Step 4: Compute the delta, or estimated improvement in rating
    let scores = matches
        .iter()
        .map(|matchup| {
            let (opponent_mu, opponent_phi) = to_glicko2(&matchup.opponent);

            let g = g_func(opponent_phi);
            let e = e_func(mu, opponent_mu, g);
            let s = match matchup.outcome {
                Outcome::Win => 1.0,
                Outcome::Lose => 0.0,
            };

            g * (s - e)
        })
        .sum::<f32>();
    let delta = v * scores;

    // Step 5: Determine the player's new volatility.
    // Whoo-boy. This is an involved process that goes into its own function.
    let new_volatility = iterate_new_volatility(v, delta, player, config.tau);

    // Step 6: Calculate pre-rating period value.
    let pre_rating_period_value =
        calculate_pre_rating_period_value(new_volatility, phi, fractional_period);

    // Step 7: Finalize rating changes.
    let new_phi = (pre_rating_period_value.powi(2).recip() + v.recip())
        .sqrt()
        .recip();
    let new_mu = new_phi.powi(2).mul_add(scores, mu);

    // Step 8: EXTRA! Clamp phi with a soft floor function.
    // Unfortunately, we have to convert min_deviation from ELO to Glicko2
    // scale...
    let new_phi = soft_floor_phi(new_phi, phi, to_phi(config.min_deviation));

    Rating {
        user_id: player.user_id,
        rating: new_mu.mul_add(173.7178, 1500.0),
        deviation: new_phi * 173.7178,
        extra: Glicko2Data {
            volatility: new_volatility,
        },
    }
}

/// Applies a soft lower limit to a Glicko-2 deviation by dampening the change
/// to phi.
fn soft_floor_phi(new_phi: f32, old_phi: f32, floor: f32) -> f32 {
    // Skip if phi increases
    if floor <= 0.0 || new_phi >= old_phi {
        return new_phi;
    }

    if old_phi <= floor {
        return new_phi.max(old_phi.min(floor).max(0.0));
    }

    let resistance = ((old_phi - floor) / old_phi).clamp(0.0, 1.0);
    old_phi - (old_phi - new_phi) * resistance
}

// We can get a rough estimate of what it would like if the player
// continued performing like this for the rest of the period, allowing us
// to instantly update the mmr!
//
// See the Lichess implementation here:
// https://github.com/lichess-org/lila/blob/d6a175d25228b0f3d9053a30301fce90850ceb2d/modules/rating/src/main/java/glicko2/RatingCalculator.java#L316
fn calculate_pre_rating_period_value(new_volatility: f32, phi: f32, fractional_period: f32) -> f32 {
    (phi.powi(2) + fractional_period * new_volatility.powi(2)).sqrt()
}

// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠙⠻⣶⣄⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣠⢦⣶⣯⣓⢚⠻⢿⣶⡤⢒⡰⠴⣦⣄⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⣴⣾⣯⡭⡕⠰⣈⠆⣉⠒⣄⠢⡹⢭⣿⡴⣈⡙⠦⣀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢰⣾⣿⣿⢋⢒⡰⢈⠵⣄⠚⡤⢩⢄⢓⡰⡁⢎⠻⣵⡜⣌⢫⣇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⡀⡸⢋⣥⡿⢋⡔⣊⡴⡍⣶⣧⡍⡴⢧⣊⠖⡰⣉⠦⡙⠼⣷⣈⠦⢻⡦⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⡠⠖⢛⣿⠱⣽⣿⢡⣳⣾⣏⣾⣽⣿⣿⣿⣾⣷⣽⣮⠱⣌⢖⣫⡳⢼⡆⢯⡱⢻⡵⡄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣴⣿⡯⣽⣿⣿⣳⣿⠿⠿⠻⠿⡻⢻⣿⣿⣿⣻⢿⣟⡜⣎⢶⣻⣗⢾⣡⡟⣭⢷⣻⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⣾⣿⣿⣿⣿⣿⣿⡼⠇⠀⠀⠀⠀⠁⠁⠉⠻⡟⢿⣿⣿⣯⣽⣷⣞⣿⣷⢻⣿⣷⣫⣿⣇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⢰⡏⣸⣿⣿⣿⣿⣿⡇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠈⠀⠹⡞⠿⣿⣿⣿⣿⣯⡟⣿⣿⣷⣿⣿⡄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠼⡂⣿⣿⣿⣿⣟⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠸⣿⣿⣿⣿⣽⣻⣿⣿⣿⡟⠃⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠸⣿⣿⣿⡟⠀⢀⢀⡀⠀⠀⠀⠀⠀⠀⠈⠉⠉⠉⠉⠁⠐⠉⣿⣿⣿⣞⣿⢿⣿⣿⡇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢹⣿⣿⠀⠀⠀⠀⣀⠀⠆⠀⠀⠀⠀⠀⠀⢁⡰⠆⠀⠀⠀⣿⣿⣿⣿⣿⡎⣿⣿⠇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣿⣿⡏⠀⣴⡾⣦⣍⠘⣆⠀⠀⠀⠀⠀⣴⠛⢹⣿⡲⠀⢽⣿⣿⣿⢋⢱⣿⠋⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⡿⣿⣿⠘⠏⠀⢻⡏⠀⠀⡄⠀⠀⠀⠀⠙⠀⠘⠍⠀⠀⡸⣿⣟⣿⡬⣼⣿⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠘⣿⣿⣇⠈⠀⠂⠀⠀⠀⡇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢠⠛⠘⣩⣴⣿⡛⠃⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠻⣿⣿⢕⠀⠀⠀⠀⠀⡇⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣿⣿⣿⣿⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠉⠀⠑⡀⠀⠀⠀⣸⣇⢠⣀⡀⠀⠀⠀⠀⠀⠀⠀⠀⡠⣿⣿⣿⠁⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠘⠀⠀⠀⠈⠙⠣⠋⠀⠀⠀⠀⠀⠀⠀⠀⣰⠙⣿⢿⡌⠃⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠐⡀⠈⠀⠒⠒⠂⠀⠀⠄⠊⠀⠀⢀⢮⠃⠀⠃⢸⣿⣆⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠐⠄⠀⠙⠃⠀⠀⠀⠀⠀⣠⠒⣭⠆⠀⠀⠀⣸⣿⣿⣦⣄⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢈⣦⣀⠀⠀⠀⢀⣤⠚⡥⢋⡜⠁⠀⠀⢀⣿⣿⣿⣿⣿⣧⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣤⣾⣿⢿⢫⡝⣩⢋⠴⣉⡶⠏⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣦⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣤⣾⣿⣿⡟⠀⢑⢾⣠⢋⣶⠉⠀⠀⠀⠀⠀⠀⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣤⡀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⠀⣀⣴⣾⣿⣿⣿⣿⣿⠃⠀⠈⡆⢷⣘⠆⢀⡄⠀⠀⠀⠀⢰⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣦⣀⠀⠀⠀⠀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⠀⠀⠀⢀⣠⣴⣾⣿⣿⣿⣿⣿⣿⣿⣿⢀⠀⢰⣷⠈⡞⢀⣾⡿⠂⠀⠀⠀⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⣦⣀⡀⠀⠀⠀⠀⠀
// ⠀⠀⠀⠀⠀⠀⣀⣤⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡏⢘⠂⡞⣿⣧⣶⣾⣿⠁⠀⠄⠀⢠⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣷⡄⠀⠀⠀
// ⠀⠀⠀⠀⠀⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇⢨⡰⠁⢿⣿⣷⣻⢾⠋⠀⠈⠄⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡀⠀⠀
// ⠀⠀⠀⠀⠀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁⠘⠕⠡⠈⢿⣷⣯⣟⠀⠀⠀⢁⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣯⠀⠀
// ⠀⠀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠀⠀⠀⠀⠀⢸⣿⣿⣿⡀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇⠀
// ⠀⠀⠀⠀⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡟⠘⠀⠀⠀⠀⣼⡿⣿⣯⣷⠀⠀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠀
// ⠀⠀⠀⠀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣧⠁⠀⠀⠀⠀⣿⣿⣯⢷⣯⡧⢀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇
// ⠀⠀⠀⢰⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇⠀⠀⠀⠀⢀⣿⢿⣽⣻⡾⣷⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠀⠀⠀⠀⠀⣼⣟⡿⣞⣷⢿⣻⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⠀⠀⣸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⠀⠀⠀⠀⣸⣿⢯⣿⢿⣽⡿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⠀⠀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇⠀⠀⠀⢀⣹⣿⣿⣿⣿⢿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⠀⢠⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇⠀⠀⠀⣈⣿⣿⣿⣷⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠆⠀⠀⠀⣽⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⠀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠃⠀⠀⡸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⠀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠃⠀⢰⣽⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⢠⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠃⢀⡷⣾⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠁⣼⡽⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⣸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⢯⣶⣟⣼⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿
// ⠀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⢯⡽⣞⣽⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿
// ⠀⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣟⣯⠾⣝⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇
// ⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡿⣼⣻⣭⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡇
// ⢸⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⡽⣶⣳⡽⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⣿⠃
// -------------------------------------------------------------
//
//                         HORRIFYING!
//
fn iterate_new_volatility(v: f32, delta: f32, player: &Glicko2Rating, tau: f32) -> f32 {
    let (_, phi) = to_glicko2(player);
    let phi_squared = phi.powi(2);

    let delta_squared = delta.powi(2);

    // Step 1: Find a. Okay, reasonable enough. Here it is.
    let mut a = f32::ln(player.volatility.powi(2));

    // Also define f. What the fuck.
    let f = move |x| {
        let x_exp = f32::exp(x);

        let tmp_1 = x_exp * (delta_squared - phi_squared - v - x_exp);
        let tmp_2 = 2.0 * (phi_squared + v + x_exp).powi(2);
        let tmp_3 = x - a;
        let tmp_4 = tau.powi(2);

        tmp_1 / tmp_2 - tmp_3 / tmp_4
    };

    // Step 2: Set iteration initial conditions.
    let mut b = if delta_squared > phi_squared + v {
        f32::ln(delta_squared - phi_squared - v)
    } else {
        let mut k = 1.0f32;

        while f(a - k * tau) < 0.0 {
            k += 1.0;
        }

        a - k * tau
    };

    // Step 3: Set f(A) and f(B) (where A and B are the initial values of a and
    // b). There is no turning back now.
    let mut f_a = f(a);
    let mut f_b = f(b);

    while (b - a).abs() > CONVERGENCE_TOLERANCE {
        let c = a + (a - b) * f_a / (f_b - f_a);
        let f_c = f(c);

        if f_c * f_b <= 0.0 {
            a = b;
            f_a = f_b;
        } else {
            f_a /= 2.0;
        }

        b = c;
        f_b = f_c;
    }

    f32::exp(a / 2.0)
}

fn e_func(mu: f32, opponent_mu: f32, g: f32) -> f32 {
    (1.0 + f32::exp(-g * (mu - opponent_mu))).recip()
}

fn g_func(phi: f32) -> f32 {
    (1.0 + 3.0 * phi.powi(2) / PI.powi(2)).sqrt().recip()
}

fn to_phi(deviation: f32) -> f32 {
    deviation / 173.7178
}

fn to_glicko2<T>(player: &Rating<T>) -> (f32, f32) {
    let mu = (player.rating - 1500.0) / 173.7178; // Glicko-2 rating
    let phi = to_phi(player.deviation); // Glicko-2 deviation

    (mu, phi)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_player_rating() -> Glicko2Rating {
        Glicko2Rating {
            user_id: 1,
            rating: 1500.0,
            deviation: 350.0,
            extra: Glicko2Data { volatility: 0.06 },
        }
    }

    /// Test taken directly from the Glicko-2 specification.
    /// <https://www.glicko.net/glicko/glicko2.pdf>
    #[test]
    fn test_glicko2() {
        let config = Glicko2Config {
            min_deviation: 0.0,
            ..Glicko2Config::default()
        };

        let player = Glicko2Rating {
            rating: 1500.0,
            deviation: 200.0,
            extra: Glicko2Data { volatility: 0.06 },
            ..new_player_rating()
        };

        let matchups = vec![
            Matchup {
                opponent: Glicko2Rating {
                    rating: 1400.0,
                    deviation: 30.0,
                    extra: Glicko2Data { volatility: 0.06 },
                    ..new_player_rating()
                },
                outcome: Outcome::Win,
            },
            Matchup {
                opponent: Glicko2Rating {
                    rating: 1550.0,
                    deviation: 100.0,
                    extra: Glicko2Data { volatility: 0.06 },
                    ..new_player_rating()
                },
                outcome: Outcome::Lose,
            },
            Matchup {
                opponent: Glicko2Rating {
                    rating: 1700.0,
                    deviation: 300.0,
                    extra: Glicko2Data { volatility: 0.06 },
                    ..new_player_rating()
                },
                outcome: Outcome::Lose,
            },
        ];

        let rating = rate(&config, &player, &matchups, 1.0);

        assert!((rating.rating - 1464.06).abs() < 0.01);
        assert!((rating.deviation - 151.52).abs() < 0.01);
        assert!((rating.volatility * 1_000_000.0 - 0_059_990.0).abs() < 0_000_010.0);
    }

    #[test]
    fn test_soft_floor() {
        let floor = 60.0;

        let reduced = soft_floor_phi(to_phi(150.0), to_phi(200.0), to_phi(floor));
        assert!((reduced * 173.7178 - 165.0).abs() < 0.5);

        let near = soft_floor_phi(to_phi(58.0), to_phi(62.0), to_phi(floor));
        assert!(near * 173.7178 > 61.8 && near * 173.7178 < 62.0);

        let up = soft_floor_phi(to_phi(80.0), to_phi(60.0), to_phi(floor));
        assert!((up * 173.7178 - 80.0).abs() < 1e-3);

        let off = soft_floor_phi(to_phi(50.0), to_phi(200.0), 0.0);
        assert!((off * 173.7178 - 50.0).abs() < 1e-3);
    }

    #[tokio::test]
    async fn test_quality() {
        let model = Glicko2::new(Glicko2Config::default());

        let p1 = model.create_rating(1).await.unwrap();
        let p2 = model.create_rating(2).await.unwrap();
        let even = model.quality(&[p1, p2]).await.unwrap();
        assert!(
            (even - 1.0).abs() < 1e-3,
            "even match should be ~1.0, got {even}"
        );

        // mismatched ratings should drop quality sharply
        let mut p3 = model.create_rating(3).await.unwrap();
        p3.rating = 1900.0;
        p3.deviation = 60.0;

        let mut p4 = model.create_rating(4).await.unwrap();
        p4.deviation = 60.0;

        let mismatch = model.quality(&[p3, p4]).await.unwrap();
        assert!(
            mismatch < 0.5,
            "mismatch should fall below 0.5, got {mismatch}"
        );
    }
}
