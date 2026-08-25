//! Short ID generation.
//!
//! For some models that will probably pass users' hands a lot, such as in
//! tables, we want to have an ID that is:
//! * **Random**  
//!   No sequences, so users have a harder time inspecting and
//!   reverse-engineering the system.
//! * **"Pretty"**  
//!   A sequential ID is pretty, but a Discord snowflake is not. Keep things
//!   short.
//! * **Fixed-Length**  
//!   Fits in ASCII tables better.

use std::{future::Future, pin::Pin};

use derive_more::Display;
use rand::{Rng, RngExt as _, SeedableRng, distr::Alphanumeric, rngs::StdRng};
use sqlx::SqliteConnection;

use crate::error::Error;

pub const MAX_INSERT_ATTEMPTS: usize = 8;

/// A helper struct used to generate an ID and insert a record until a unique
/// constraint is not violated.
#[derive(Debug)]
pub struct IdAllocator<R> {
    rng: R,
    length: usize,
    max_insert_attempts: usize,
}

impl IdAllocator<()> {
    /// Creates a new `IdAllocator` using the thread local RNG.
    pub fn new() -> IdAllocator<StdRng> {
        let rng = StdRng::seed_from_u64(rand::random());
        IdAllocator::new_with(rng)
    }
}

impl<R> IdAllocator<R> {
    /// Creates a new `IdAllocator` using the given RNG.
    pub fn new_with(rng: R) -> IdAllocator<R> {
        IdAllocator {
            rng,
            length: 6,
            max_insert_attempts: MAX_INSERT_ATTEMPTS,
        }
    }

    /// Sets the length of the ID.
    ///
    /// The default is `6`.
    pub fn length(self, length: usize) -> IdAllocator<R> {
        IdAllocator { length, ..self }
    }

    /// Sets how many times an insert is attempted before giving up.
    ///
    /// The default is [`MAX_INSERT_ATTEMPTS`].
    pub fn max_insert_attempts(self, length: usize) -> IdAllocator<R> {
        IdAllocator { length, ..self }
    }
}

impl<R> IdAllocator<R>
where
    R: Rng,
{
    /// Attempts to insert a record with a given function.
    pub async fn insert<F, T>(
        &mut self,
        conn: &mut SqliteConnection,
        mut func: F,
    ) -> Result<T, Error>
    where
        F: for<'a> FnMut(
            &'a str,
            &'a mut SqliteConnection,
        ) -> Pin<Box<dyn Future<Output = Result<T, sqlx::Error>> + Send + 'a>>,
        T: Sized,
    {
        let mut inserted_entity = None::<T>;

        for _ in 0..self.max_insert_attempts {
            // generate ID
            let short_id = generate_id_with(self.length, &mut self.rng);

            // try to insert
            match func(&short_id, &mut *conn).await {
                Ok(entity) => {
                    inserted_entity = Some(entity);
                    break;
                }
                Err(sqlx::Error::Database(err)) if err.is_unique_violation() => {
                    // short id unique violation, try again
                    tracing::debug!("unique key {} failed, regenerating", short_id)
                }
                Err(err) => return Err(Error::new(err)),
            }
        }

        inserted_entity.ok_or_else(|| IdsExhausted.into())
    }
}

/// A helper function used to generate an ID and insert a record until a unique
/// constraint is not violated.
pub fn allocate() -> IdAllocator<StdRng> {
    IdAllocator::new()
}

/// A helper function used to generate an ID and insert a record until a unique
/// constraint is not violated.
pub fn allocate_with<R>(rng: R) -> IdAllocator<R> {
    IdAllocator::new_with(rng)
}

/// Generates a short ID.
pub fn generate_id<R>(length: usize) -> String {
    let mut rng = rand::rng();
    generate_id_with(length, &mut rng)
}

/// Generates a short ID with a given RNG.
pub fn generate_id_with<R>(length: usize, rng: &mut R) -> String
where
    R: Rng,
{
    rng.sample_iter(Alphanumeric)
        .take(length)
        .map(char::from)
        .map(|c| char::to_ascii_uppercase(&c))
        .collect::<String>()
}

/// An error for when a suitable ID cannot be generated.
#[derive(Debug, Display)]
#[display("cannot create object; ran out of ids")]
pub struct IdsExhausted;

impl std::error::Error for IdsExhausted {}
