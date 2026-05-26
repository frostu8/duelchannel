//! OAuth Authorization Grant flow.

use eyre::Error;

use oauth2::{
    AuthUrl, ClientId, ClientSecret, EndpointNotSet, EndpointSet, RedirectUrl, RevocationUrl,
    TokenUrl, basic::BasicClient,
};
use sqlx::SqlitePool;

use std::sync::Arc;

use crate::config::DiscordConfig;

pub use crate::session::Session;

/// The base url to authorize with Discord.
pub const DISCORD_AUTHORIZATION_URL: &str = "https://discord.com/oauth2/authorize";

/// The url used to fetch tokens from Discord.
pub const DISCORD_TOKEN_URL: &str = "https://discord.com/api/oauth2/token";

/// The url to revoke tokens.
pub const DISCORD_REVOCATION_URL: &str = "https://discord.com/api/oauth2/token/revoke";

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

type OauthClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointSet, EndpointSet>;

/// Additional OAuth state.
///
/// Cheaply cloneable.
#[derive(Clone, Debug)]
pub struct OauthState {
    pub db: SqlitePool,
    /// The client used to access the oauth state.
    pub client: OauthClient,
    /// The http reqwest client used to make requests.
    pub http_client: reqwest::Client,
    /// The URL to redirect to after a successful authorization code grant.
    pub redirect_to: Option<Arc<str>>,
}

impl OauthState {
    /// Creates a new `OauthState` from a config.
    pub fn new(
        base_url: impl AsRef<str>,
        db: SqlitePool,
        config: &DiscordConfig,
    ) -> Result<OauthState, Error> {
        let base_url = base_url.as_ref();
        let redirect_url = format!("{}/auth/~login", base_url);

        let client = BasicClient::new(ClientId::new(config.client_id.to_string()))
            .set_client_secret(ClientSecret::new(config.client_secret.clone()))
            .set_auth_uri(AuthUrl::new(DISCORD_AUTHORIZATION_URL.to_owned())?)
            .set_token_uri(TokenUrl::new(DISCORD_TOKEN_URL.to_owned())?)
            .set_revocation_url(RevocationUrl::new(DISCORD_REVOCATION_URL.to_owned())?)
            .set_redirect_uri(RedirectUrl::new(redirect_url)?);

        let http_client = reqwest::Client::builder()
            // Following redirects opens the client up to SSRF vulnerabilities.
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(USER_AGENT)
            .build()?;

        Ok(OauthState {
            db,
            client,
            http_client,
            redirect_to: None,
        })
    }

    /// Sets the `redirect_to`.
    pub fn with_redirect_to(self, redirect_to: impl Into<Option<String>>) -> OauthState {
        OauthState {
            redirect_to: redirect_to.into().map(|s| Arc::from(s)),
            ..self
        }
    }
}
