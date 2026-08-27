//! Integration tests for the skill-rating (MMR) entity layer.

use chrono::{DateTime, TimeDelta, Utc};
use duelchannel::entity::user::mmr::{get_rating, init_rating_at, update_ratings_at};
use duelchannel::mmr::RatingModel;
use duelchannel::mmr::glicko2::{Glicko2, Glicko2Config};
use sqlx::SqlitePool;

/// Fixed test epoch.
fn epoch() -> DateTime<Utc> {
    DateTime::from_timestamp(1_757_000_000, 0).unwrap()
}

/// Opens a migrated in-memory pool.
async fn setup_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("open in-memory pool");

    sqlx::migrate!().run(&pool).await.expect("apply migrations");

    pool
}

/// Inserts a user + profile.
async fn insert_user(pool: &SqlitePool, short: &str, name: &str) -> i32 {
    let now = epoch();
    let res = sqlx::query(
        r#"
        INSERT INTO user (short_id, display_name, inserted_at, updated_at)
        VALUES ($1, $2, $3, $3)
        "#,
    )
    .bind(format!("{short:<6}"))
    .bind(name)
    .bind(now)
    .execute(pool)
    .await
    .expect("insert user");

    let id = res.last_insert_rowid() as i32;

    sqlx::query(
        r#"
        INSERT INTO profile (id, parent_id, public_key, inserted_at, updated_at)
        VALUES ($1, $2, $3, $4, $4)
        "#,
    )
    .bind(id)
    .bind(id)
    .bind(vec![id as u8; 32])
    .bind(now)
    .execute(pool)
    .await
    .expect("insert profile");

    id
}

/// Inserts a concluded 1v1 battle between winner and loser.
async fn insert_battle(pool: &SqlitePool, winner: i32, loser: i32, at: chrono::DateTime<Utc>) {
    let res = sqlx::query(
        r#"
        INSERT INTO battle (uuid, level_name, status, concluded_at, inserted_at, updated_at)
        VALUES ($1, 'map', 1, $2, $3, $3)
        "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(at)
    .bind(at)
    .execute(pool)
    .await
    .expect("insert battle");

    let id = res.last_insert_rowid() as i32;

    // Winner
    sqlx::query(
        r#"
        INSERT INTO participant (match_id, profile_id, user_id, name, team, finish_time, no_contest)
        VALUES ($1, $2, $3, 'pw', 0, 1000, FALSE)
        "#,
    )
    .bind(id)
    .bind(winner)
    .bind(winner)
    .execute(pool)
    .await
    .expect("insert winner participant");

    // Loser
    sqlx::query(
        r#"
        INSERT INTO participant (match_id, profile_id, user_id, name, team, finish_time, no_contest)
        VALUES ($1, $2, $3, 'pl', 0, NULL, TRUE)
        "#,
    )
    .bind(id)
    .bind(loser)
    .bind(loser)
    .execute(pool)
    .await
    .expect("insert loser participant");
}

/// The user's ordinal as would be displayed in the API.
async fn user_ordinal(pool: &SqlitePool, user_id: i32) -> i32 {
    sqlx::query_as::<_, (i32,)>("SELECT ordinal FROM user WHERE id = $1")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map(|(id,)| id)
        .expect("user ordinal")
}

/// #rating_period rows.
async fn period_count(pool: &SqlitePool) -> i32 {
    sqlx::query_as::<_, (i32,)>("SELECT COUNT(*) FROM rating_period")
        .fetch_one(pool)
        .await
        .expect("count periods")
        .0
}

/// #concluded battles.
async fn concluded_battle_count(pool: &SqlitePool) -> i32 {
    sqlx::query_as::<_, (i32,)>(
        "SELECT COUNT(*) FROM battle WHERE status = 1 AND concluded_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .expect("count battles")
    .0
}

/// Basic test; does the user's ordinal update when winning?
#[tokio::test]
async fn update_ordinal_after_win() {
    let pool = setup_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    let model = Glicko2::new(Glicko2Config::default());
    let t0 = epoch();

    let winner = insert_user(&pool, "WNR123", "winner").await;
    let loser = insert_user(&pool, "LSR456", "loser").await;

    init_rating_at(winner, t0, &model, &mut conn)
        .await
        .expect("init winner");
    init_rating_at(loser, t0, &model, &mut conn)
        .await
        .expect("init loser");

    let before_ordinal = user_ordinal(&pool, winner).await;

    // Battle concludes just inside period 1.
    // let t_battle = t0 + TimeDelta::seconds(10);
    let t_battle = t0;
    insert_battle(&pool, winner, loser, t_battle).await;
    assert_eq!(concluded_battle_count(&pool).await, 1);

    update_ratings_at(
        &[winner],
        &model,
        t_battle + TimeDelta::seconds(10),
        &mut conn,
    )
    .await
    .expect("update_ratings_at");

    let after_ordinal = user_ordinal(&pool, winner).await;
    assert_ne!(after_ordinal, before_ordinal, "ordinal should change");

    let period_count = period_count(&pool).await;
    assert!(
        period_count == 1,
        "new period should not have been made, got {}",
        period_count
    );
}

/// On a rating-period rollover, a new period must be created and cataloging
/// must catalog the row under it.
#[tokio::test]
async fn rollover_creates_new_period_and_catalogs_it() {
    let pool = setup_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    let model = Glicko2::new(Glicko2Config::default());
    let t0 = epoch();

    let winner = insert_user(&pool, "WNR123", "winner").await;
    let loser = insert_user(&pool, "LSR456", "loser").await;

    init_rating_at(winner, t0, &model, &mut conn)
        .await
        .expect("init winner");
    init_rating_at(loser, t0, &model, &mut conn)
        .await
        .expect("init loser");

    // Battle inside period 1 at t0+10s.
    insert_battle(&pool, winner, loser, t0 + TimeDelta::seconds(10)).await;

    // One full rating period later, another battle.
    let t1 = t0 + model.period();
    insert_battle(&pool, winner, loser, t1).await;

    update_ratings_at(&[winner, loser], &model, t1, &mut conn)
        .await
        .expect("update_ratings_at");

    let period_count = period_count(&pool).await;
    assert!(
        period_count >= 2,
        "rollover must create a new period, got {}",
        period_count
    );

    let w = get_rating::<Glicko2>(winner, t1, &model, &mut conn)
        .await
        .unwrap();

    assert!(
        w.deviation < 350.0,
        "idle player deviation should go down, got {}",
        w.deviation,
    );
    assert!(
        w.period.started_at >= t0 + model.period(),
        "last rating row belongs to the new period ({:?})",
        w.period
    );
}

/// Idle players should have their deviation regrow across closed periods, not
/// be frozen at period snapshot.
#[tokio::test]
async fn idle_player_decays_through_closed_periods() {
    let pool = setup_pool().await;
    let mut conn = pool.acquire().await.unwrap();
    let mut cfg = Glicko2Config::default();
    cfg.decay_grace = cfg.period * 2;
    let model = Glicko2::new(cfg);
    let t0 = epoch();

    let winner = insert_user(&pool, "WNR123", "winner").await;
    let loser = insert_user(&pool, "LSR456", "loser").await;
    let idler = insert_user(&pool, "IDL789", "idler").await;

    for u in [winner, loser, idler] {
        init_rating_at(u, t0, &model, &mut conn)
            .await
            .expect("init");
    }

    // In case we decide to add a hard upper limmit on deviation, have the
    // idler play one game
    insert_battle(&pool, winner, idler, t0 + TimeDelta::seconds(15)).await;

    let t1 = t0 + model.period();
    update_ratings_at(&[winner, loser, idler], &model, t1, &mut conn)
        .await
        .expect("initial update");

    let idler_row = get_rating::<Glicko2>(idler, t1, &model, &mut conn)
        .await
        .unwrap();

    assert!(
        idler_row.deviation < 350.0,
        "idle player deviation should go down, got {}",
        idler_row.deviation,
    );

    let before_deviation = idler_row.deviation;

    // Do many battles without idler participating
    let mut time = t1 + model.period();
    for i in 0..8 {
        insert_battle(&pool, winner, loser, time).await;
        update_ratings_at(&[winner, loser, idler], &model, time, &mut conn)
            .await
            .expect("rollover");

        time += model.period();

        if i < 2 {
            let idler_row = get_rating::<Glicko2>(idler, time, &model, &mut conn)
                .await
                .unwrap();

            assert_eq!(
                idler_row.deviation, before_deviation,
                "idle player deviation should stay the same",
            );
        }
    }

    let idler_row = get_rating::<Glicko2>(idler, time, &model, &mut conn)
        .await
        .unwrap();

    assert!(
        idler_row.deviation > before_deviation,
        "idle player deviation should regrow, got {} <= {}",
        idler_row.deviation,
        before_deviation,
    );
}

/// Test for unexpected DR drops.
#[tokio::test]
async fn unexpected_dr_drops() {
    const BATTLE_COUNT: i32 = 50;

    let pool = setup_pool().await;
    let mut conn = pool.acquire().await.unwrap();

    let cfg = Glicko2Config::default();
    let model = Glicko2::new(cfg.clone());
    let t0 = epoch();

    let winner = insert_user(&pool, "WNR123", "winner").await;
    let loser = insert_user(&pool, "LSR456", "loser").await;

    init_rating_at(winner, t0, &model, &mut conn)
        .await
        .expect("init winner");
    init_rating_at(loser, t0, &model, &mut conn)
        .await
        .expect("init loser");

    // Lets get battling!! This guy is gonna get slimed.
    let mut time = t0;
    let step = cfg.period / BATTLE_COUNT;

    let mut winner_ordinal = user_ordinal(&pool, winner).await;
    let mut loser_ordinal = user_ordinal(&pool, loser).await;

    for i in 0..BATTLE_COUNT {
        insert_battle(&pool, winner, loser, time).await;
        update_ratings_at(
            &[winner, loser],
            &model,
            time + TimeDelta::seconds(10),
            &mut conn,
        )
        .await
        .expect("update_ratings_at");

        assert_eq!(concluded_battle_count(&pool).await, i + 1);

        let new_winner_ordinal = user_ordinal(&pool, winner).await;
        let new_loser_ordinal = user_ordinal(&pool, loser).await;

        assert!(
            new_winner_ordinal > winner_ordinal,
            "#{} winner ordinal should be higher, {} -> {}",
            i,
            winner_ordinal,
            new_winner_ordinal
        );
        assert!(
            new_loser_ordinal < loser_ordinal,
            "#{} loser ordinal should be lower, {} -> {}",
            i,
            loser_ordinal,
            new_loser_ordinal
        );

        winner_ordinal = new_winner_ordinal;
        loser_ordinal = new_loser_ordinal;

        time += step;
    }

    let pc = period_count(&pool).await;
    assert_eq!(pc, 1, "rollover shouldn't happen yet");

    // Add another battle
    let t1 = t0 + cfg.period;
    insert_battle(&pool, winner, loser, t1).await;

    update_ratings_at(
        &[winner, loser],
        &model,
        t1 + TimeDelta::seconds(10),
        &mut conn,
    )
    .await
    .expect("update_ratings_at");

    let pc = period_count(&pool).await;
    assert!(pc >= 2, "rollover must create a new period, got {}", pc);

    let new_winner_ordinal = user_ordinal(&pool, winner).await;
    let new_loser_ordinal = user_ordinal(&pool, loser).await;

    assert!(
        new_winner_ordinal >= winner_ordinal,
        "winner ordinal should be higher or stay the same, {} -> {}",
        winner_ordinal,
        new_winner_ordinal
    );
    assert!(
        new_loser_ordinal <= loser_ordinal,
        "loser ordinal should be lower or stay the same, {} -> {}",
        loser_ordinal,
        new_loser_ordinal
    );

    winner_ordinal = new_winner_ordinal;
    loser_ordinal = new_loser_ordinal;

    // Add ANOTHER battle
    let t2 = t1 + TimeDelta::seconds(10);
    insert_battle(&pool, winner, loser, t2).await;

    update_ratings_at(
        &[winner, loser],
        &model,
        t2 + TimeDelta::seconds(10),
        &mut conn,
    )
    .await
    .expect("update_ratings_at");

    assert!(
        new_winner_ordinal >= winner_ordinal,
        "winner ordinal should be higher or stay the same, {} -> {}",
        winner_ordinal,
        new_winner_ordinal
    );
    assert!(
        new_loser_ordinal <= loser_ordinal,
        "loser ordinal should be lower or stay the same, {} -> {}",
        loser_ordinal,
        new_loser_ordinal
    );
}
