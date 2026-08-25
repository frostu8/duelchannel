//! OpenAPI documentation generation.

use utoipa::OpenApi;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};

use duelchannel_model::{
    ApiError, CurrentUser, Profile, User,
    battle::{Battle, BattlePoint, BattleStatistics, BattleStatus, Participant, PlayerTeam},
    profile::Skin,
    request::{
        battle::{
            CreateBattleParticipant, CreateBattleRequest, UpdateBattleRequest,
            UpdatePlayerPlacementRequest,
        },
        user::{CreateUser, CreateUserProfile},
    },
    server::{BannedStatus, MapConfig, Server, SkillRange},
};

use crate::routes;

/// The OpenAPI document for the Duel Channel API.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "Duel Channel API",
        description = "Access API for the Duel Channel Ring Racers server.",
        license(name = "CC0-1.0", url = "https://creativecommons.org/publicdomain/zero/1.0/"),
    ),
    paths(
        routes::battle::list,
        routes::battle::show,
        routes::battle::create,
        routes::battle::update,
        routes::battle::player::update,
        routes::battle::replay::upload,
        routes::battle::analytics::show,
        routes::player::create,
        routes::player::list,
        routes::player::show_self,
        routes::player::show,
        routes::server::list,
        routes::server::show_self,
        routes::server::update_self,
        routes::auth::redirect,
        routes::auth::login,
    ),
    components(schemas(
        ApiError,
        Battle,
        BattlePoint,
        BattleStatistics,
        BattleStatus,
        Participant,
        PlayerTeam,
        Skin,
        User,
        CurrentUser,
        Profile,
        Server,
        MapConfig,
        SkillRange,
        BannedStatus,
        CreateBattleRequest,
        CreateBattleParticipant,
        UpdateBattleRequest,
        UpdatePlayerPlacementRequest,
        CreateUser,
        CreateUserProfile,
        routes::server::UpdateServerRequest,
        routes::server::UpdateMapConfig,
        routes::server::SkillRange,
    )),
    tags(
        (name = "match", description = "Match-related operations."),
        (name = "player", description = "Endpoints that operate on players."),
        (name = "server", description = "Endpoints for servers."),
        (name = "auth", description = "User authentication via Discord OAuth2."),
    ),
    modifiers(&SecurityAddon),
)]
pub struct ApiDoc;

/// Adds the security schemes to the document.
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            "apiKey",
            SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("X-API-KEY"))),
        );
        components.add_security_scheme(
            "cookie",
            SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::new("id"))),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::ApiDoc;
    use utoipa::OpenApi as _;

    #[test]
    fn openapi_generates_and_covers_routes() {
        let doc = ApiDoc::openapi();
        let json = serde_json::to_string_pretty(&doc).expect("serializes to JSON");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        let paths = value["paths"].as_object().expect("paths object");

        let expected = [
            "/matches",
            "/matches/{battle_id}",
            "/matches/{battle_id}/players/{short_id}",
            "/matches/{battle_id}/replay",
            "/matches/analytics",
            "/players",
            "/players/~me",
            "/players/{short_id}",
            "/servers",
            "/servers/~me",
            "/auth/~redirect",
            "/auth/~login",
        ];

        for path in expected {
            assert!(paths.contains_key(path), "missing path: {path}");
        }

        // Both operations on /matches
        let matches = &paths["/matches"];
        assert!(matches.get("get").is_some(), "GET /matches missing");
        assert!(matches.get("post").is_some(), "POST /matches missing");

        let schemas = value["components"]["schemas"]
            .as_object()
            .expect("schemas object");

        for schema in [
            "Battle",
            "Participant",
            "BattleStatus",
            "PlayerTeam",
            "User",
            "CurrentUser",
            "Profile",
            "Server",
            "MapConfig",
            "BannedStatus",
            "ApiError",
            "CreateBattleRequest",
            "UpdateBattleRequest",
            "CreateUser",
        ] {
            assert!(schemas.contains_key(schema), "missing schema: {schema}");
        }

        // Security schemes
        let security = value["components"]["securitySchemes"]
            .as_object()
            .expect("securitySchemes object");
        assert!(security.contains_key("apiKey"), "missing apiKey scheme");
        assert!(security.contains_key("cookie"), "missing cookie scheme");

        // serde_repr enums must render as integer enums, not strings
        let battle_status = &schemas["BattleStatus"];
        assert_eq!(
            battle_status["type"], "integer",
            "BattleStatus should be an integer enum"
        );
    }
}
