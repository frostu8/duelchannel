//! OAuth sessions.

use axum::{
    RequestPartsExt as _,
    extract::{FromRef, FromRequestParts},
};

use cookie::{Cookie, SameSite};

use derive_more::{Deref, DerefMut};

use duelchannel_model::CurrentUser;

use time::Duration;

use tower_cookies::Cookies;

use std::{
    borrow::Cow,
    fmt::{self, Debug, Formatter},
};

use http::request::Parts;

use rand::{Rng, distr::Distribution};

use serde::{Deserialize, Serialize};

use tower_sessions::Session as TowerSession;

use crate::{
    app::AppState,
    error::{Error, ErrorKind},
    schema::user::get_user,
};

pub type SessionError = tower_sessions::session::Error;

/// A session, used to keep state.
///
/// **Warning!** These sessions are short-lived and are simply to carry some
/// basic information about the client.
///
/// These are not for credentials!
#[derive(Clone, Deref)]
pub struct Session {
    session: TowerSession,
    // too cute of a name
    cookie_jar: Cookies,
    #[deref]
    data: SessionData,
}

/// Inner session data.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionData {
    /// A randomly generated token for OAuth2 flows.
    pub state: String,
    /// A randomly generated csrf token.
    pub csrf: String,
    /// The identity of the user.
    ///
    /// This is the user's ID in the database. If this is `None`, this is an
    /// anonymous session.
    pub identity: Option<i32>,
}

impl Session {
    /// The name of the key this struct is stored in on the session.
    pub const SESSION_KEY: &'static str = "oauth_session";

    /// Sets the user of the session.
    ///
    /// **Only call this if you are confident the user has followed the proper
    /// authentication flow!**
    pub async fn set_user(&mut self, user_id: i32) -> Result<(), SessionError> {
        self.data.identity = Some(user_id);
        self.update_data().await?;

        Ok(())
    }

    /// Shuffles the CSRF token.
    ///
    /// When a mutation is finished on the server, this should always be
    /// called.
    pub async fn shuffle_csrf(&mut self) -> Result<(), SessionError> {
        self.data.csrf = generate_csrf();
        self.update_data().await?;

        let one_year = time::OffsetDateTime::now_utc() + Duration::days(365);

        // update csrf cookie
        let mut csrf_cookie = Cookie::new("csrf", Cow::Owned(self.data.csrf.clone()));
        csrf_cookie.set_http_only(false); // make sure js can access this cookie
        csrf_cookie.set_same_site(SameSite::Strict);
        csrf_cookie.set_expires(one_year);
        csrf_cookie.set_path("/");

        self.cookie_jar.add(csrf_cookie);

        Ok(())
    }

    async fn update_data(&self) -> Result<(), SessionError> {
        self.session.insert(Session::SESSION_KEY, &self.data).await
    }
}

impl Debug for Session {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("state", &self.data.csrf)
            .field("identity", &self.data.identity)
            .finish()
    }
}

impl<S> FromRequestParts<S> for Session
where
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let session = parts
            .extract::<TowerSession>()
            .await
            .map_err(ErrorKind::CookieFetch)?;
        let cookie_jar = parts
            .extract::<Cookies>()
            .await
            .map_err(ErrorKind::CookieFetch)?;

        let session_data = if let Some(session_data) = session.get(Session::SESSION_KEY).await? {
            session_data
        } else {
            // create new session
            tracing::trace!("creating new session");
            let session_data = SessionData {
                state: generate_csrf(),
                csrf: generate_csrf(),
                identity: None,
            };
            session.insert(Session::SESSION_KEY, &session_data).await?;
            session_data
        };

        Ok(Session {
            session,
            cookie_jar,
            data: session_data,
        })
    }
}

/// An authenticated user.
///
/// This type dereferences into the stored user [`User`], which stores basic
/// information about the user that is typically suitable for most endpoints.
#[derive(Clone, Debug, Deref, DerefMut)]
pub struct SessionUser {
    #[deref]
    #[deref_mut]
    user: CurrentUser,
    identity: i32,
}

impl SessionUser {
    /// Unwraps the inner user model.
    pub fn into_inner(self) -> CurrentUser {
        self.user
    }

    /// The database ID of the user.
    ///
    /// This is simply a copy of [`SessionData::identity`], but you don't have
    /// to work with an [`Option`].
    pub fn identity(&self) -> i32 {
        self.identity
    }
}

impl<S> FromRequestParts<S> for SessionUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let session = parts.extract_with_state::<Session, S>(state).await?;

        let state = AppState::from_ref(state);

        if let Some(identity) = session.identity {
            let mut conn = state.db.acquire().await?;
            let user = get_user(identity, &mut *conn).await?;

            if let Some(user) = user {
                Ok(SessionUser {
                    user: CurrentUser::from(user),
                    identity,
                })
            } else {
                Err(ErrorKind::InvalidSession.into())
            }
        } else {
            Err(ErrorKind::UserUnauthenticated.into())
        }
    }
}

/// A random distribution for base 64.
#[derive(Clone, Copy, Debug, Default)]
pub struct Base64;

impl Distribution<u8> for Base64 {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> u8 {
        const GEN_ASCII_STR_CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ\
                abcdefghijklmnopqrstuvwxyz\
                0123456789\
                -_";

        let ix = rng.next_u32() >> (32 - 6);
        GEN_ASCII_STR_CHARSET[ix as usize]
    }
}

/// Generates a random state with thread-local entropy.
pub fn generate_csrf() -> String {
    let mut rng = rand::rng();
    generate_csrf_with(&mut rng)
}

/// Generates a random state with a provided random generator.
pub fn generate_csrf_with<R>(rng: &mut R) -> String
where
    R: Rng,
{
    rng.sample_iter(Base64).take(64).map(char::from).collect()
}
