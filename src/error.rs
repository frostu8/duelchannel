//! Application error that may occur during the processing of a request.
//!
//! See [`Error`].

use std::{
    error::Error as StdError,
    fmt::{self, Display, Formatter},
    sync::Arc,
};

use axum::{
    extract::rejection::{FormRejection, JsonRejection},
    response::{IntoResponse, Response},
};

use garde::error::Report;

use derive_more::{Display, From};

use http::StatusCode;

use duelchannel_model::{ApiError, rrid::Rrid};

use uuid::Uuid;

use crate::app::AppJson;

/// Application error that may occur during the processing of a request.
///
/// This includes both internal errors and user errors.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: Option<String>,
}

impl Error {
    /// Constucts an internal error.
    pub fn new<E: StdError + Send + Sync + 'static>(e: E) -> Error {
        Error {
            kind: ErrorKind::Other(Box::new(e)),
            message: None,
        }
    }

    /// Constructs a new not found error.
    pub fn not_found(message: impl Into<String>) -> Error {
        Error {
            kind: ErrorKind::NotFound,
            message: Some(message.into()),
        }
    }

    /// Adds a custom message to the error.
    pub fn with_message(self, message: impl Into<String>) -> Error {
        Error {
            message: Some(message.into()),
            ..self
        }
    }

    /// The inner [`AppErrorKind`] of the error.
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// Discards the error message, unwrapping the inner error.
    pub fn into_kind(self) -> ErrorKind {
        self.kind
    }

    /// Checks if the error is an internal error.
    pub fn is_internal(&self) -> bool {
        matches!(
            self.kind,
            ErrorKind::Database(_)
                | ErrorKind::WebSocket(_)
                | ErrorKind::Session(_)
                | ErrorKind::HttpClient(_)
                | ErrorKind::Discord(_)
                | ErrorKind::OutOfIds
                | ErrorKind::Other(_)
        )
    }

    /// Converts the app error to an API error.
    pub fn to_api_error(self) -> ApiError {
        let (_, error) = self.to_status_and_api_error();
        error
    }

    fn to_status_and_api_error(self) -> (StatusCode, ApiError) {
        let (status, mut error) = match self.kind {
            ErrorKind::NotFound => (
                StatusCode::NOT_FOUND,
                ApiError {
                    message: "Resource not found".into(),
                },
            ),
            error_kind @ ErrorKind::AlreadyConcluded(_) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: error_kind.to_string(),
                },
            ),
            error_kind @ ErrorKind::MissingParticipant(_) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: error_kind.to_string(),
                },
            ),
            ErrorKind::Garde(error) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: error.to_string(),
                },
            ),
            ErrorKind::Json(error) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: error.to_string(),
                },
            ),
            ErrorKind::Multipart(error) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: error.to_string(),
                },
            ),
            ErrorKind::MultipartParse(error) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: error.to_string(),
                },
            ),
            ErrorKind::SerdeJson(error) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: error.to_string(),
                },
            ),
            ErrorKind::Form(error) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: error.to_string(),
                },
            ),
            ErrorKind::UnsupportedContentType(mime) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: format!("Unrecognized MIME type: {}", mime),
                },
            ),
            ErrorKind::MissingContentType => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: "Missing request content type".into(),
                },
            ),
            ErrorKind::ApiKeyUnauthenticated => (
                StatusCode::UNAUTHORIZED,
                ApiError {
                    message: "No API key passed; set an X-API-Key header!".into(),
                },
            ),
            ErrorKind::ApiKeyBadCredentials => (
                StatusCode::UNAUTHORIZED,
                ApiError {
                    message: "API key was malformed".into(),
                },
            ),
            ErrorKind::UserUnauthenticated => (
                StatusCode::UNAUTHORIZED,
                ApiError {
                    message: "User is unauthenticated".into(),
                },
            ),
            ErrorKind::InvalidSession => (
                StatusCode::UNAUTHORIZED,
                ApiError {
                    message: "Session is invalid or bad; perhaps this is an old cookie?".into(),
                },
            ),
            ErrorKind::InvalidState { .. } => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: "Invalid state sent".into(),
                },
            ),
            ErrorKind::CookieFetch((code, message)) => (
                code,
                ApiError {
                    message: message.into(),
                },
            ),
            ErrorKind::MissingHostHeader => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: "Missing Host header".into(),
                },
            ),
            ErrorKind::InvalidCsrfToken => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: "Invalid csrf token passed".into(),
                },
            ),
            ErrorKind::ProfileInUse(rrid) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: format!("profile {} already in use", rrid),
                },
            ),
            ErrorKind::InvalidData(message) => (StatusCode::BAD_REQUEST, ApiError { message }),
            // Area for errors we are ok with just using their display impl
            err @ ErrorKind::DuplicateParticipant(_) => (
                StatusCode::BAD_REQUEST,
                ApiError {
                    message: err.to_string(),
                },
            ),
            // fallthrough for internal server errors not turned into user
            // errors here
            _error_kind => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError {
                    message: "An internal server error occured".into(),
                },
            ),
        };

        // replace error message
        if let Some(message) = self.message {
            error.message = message;
        }

        (status, error)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.message.as_ref() {
            Some(msg) => f.write_str(msg),
            None => Display::fmt(&self.kind, f),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match &self.kind {
            ErrorKind::Json(err) => Some(err),
            ErrorKind::Form(err) => Some(err),
            ErrorKind::Database(err) => Some(err),
            ErrorKind::Session(err) => Some(err),
            ErrorKind::WebSocket(err) => Some(err),
            ErrorKind::HttpClient(err) => Some(err),
            ErrorKind::Discord(err) => Some(err),
            ErrorKind::Garde(err) => Some(err),
            ErrorKind::Other(err) => err.source(),
            _ => None,
        }
    }
}

impl<T> From<T> for Error
where
    T: Into<ErrorKind>,
{
    fn from(value: T) -> Self {
        Error {
            kind: value.into(),
            message: None,
        }
    }
}

/// The specific kind of error that happened.
#[derive(Debug, Display, From)]
#[non_exhaustive]
pub enum ErrorKind {
    /// The request's JSON payload was malformed or invalid.
    #[display("{_0}")]
    Json(JsonRejection),
    /// A JSON-related error occured.
    SerdeJson(serde_json::Error),
    /// The request's urlencoded payload was malformed or invalid.
    #[display("{_0}")]
    Form(FormRejection),
    /// Input validation failed.
    #[display("{_0}")]
    Garde(Report),
    /// Bad multipart data.
    #[display("{_0}")]
    Multipart(axum::extract::multipart::MultipartRejection),
    /// Bad multipart data.
    #[display("{_0}")]
    MultipartParse(axum::extract::multipart::MultipartError),
    /// A resource was not found.
    #[display("Resource not found")]
    NotFound,
    /// A battle with the given UUID already concluded.
    #[display("Battle {_0} concluded")]
    #[from(ignore)]
    AlreadyConcluded(Uuid),
    /// A user was attempted to be created with a profile that is already in
    /// use.
    #[display("profile {_0} already used")]
    #[from(ignore)]
    ProfileInUse(Rrid),
    /// A battle was attempted to be started with a bad participant.
    #[display("participant {_0} not found")]
    #[from(ignore)]
    MissingParticipant(String),
    /// A duplicate participant was attempted to be added.
    #[display("participant {_0} used many times")]
    #[from(ignore)]
    DuplicateParticipant(String),
    /// A battle was created with an unregistered profile.
    #[display("profile {_0} not found")]
    #[from(ignore)]
    MissingProfile(Rrid),
    /// A content type was not provided.
    MissingContentType,
    /// The server cannot serve this content type.
    #[from(ignore)]
    UnsupportedContentType(String),
    /// The client attempted to access a protected endpoint without an api key.
    #[display("No api key given")]
    ApiKeyUnauthenticated,
    /// The client presented bad credentials.
    #[display("Bad api key given")]
    ApiKeyBadCredentials,
    /// The client attempted to access a protected endpoint without a valid
    /// user session.
    #[display("No authentication given")]
    UserUnauthenticated,
    /// The client attempted to access a protected endpoint without a valid
    /// user session.
    #[display("Session invalid")]
    InvalidSession,
    /// An invalid csrf token was passed.
    #[display("Csrf verification failed")]
    InvalidCsrfToken,
    /// A valid schema was passed, but the data was otherwise invalid.
    #[display("{_0}")]
    #[from(ignore)]
    InvalidData(String),
    /// An error with the session occured.
    #[display("{} {}: {}", _0.0, _0.0.canonical_reason().unwrap_or("Error"), _0.1)]
    #[from(ignore)]
    CookieFetch((StatusCode, &'static str)),
    /// A host header was missing, which is used to identify the server when
    /// sending cookies.
    MissingHostHeader,
    /// An error getting session data occured.
    Session(tower_sessions::session::Error),
    /// A request to the Discord API failed.
    Discord(twilight_http::Error),
    /// An HTTP request failed.
    HttpClient(reqwest::Error),
    /// Invalid state in OAuth2 grant flow detected.
    #[from(ignore)]
    InvalidState { state: String },
    /// A websocket error occured.
    #[from(ignore)]
    WebSocket(axum::Error),
    /// Replay file is too large.
    #[display("replay too large")]
    ReplayTooLarge,
    /// An unhandled database error occured.
    Database(sqlx::Error),
    /// The application failed to generate a unique id.
    #[display("Ran out of ids")]
    OutOfIds,
    /// An error happened.
    ///
    /// Only the message is preserved! All errors of this kind are internal.
    /// Use as a last resort.
    #[from(ignore)]
    Other(Box<dyn StdError + Send + Sync + 'static>),
}

impl IntoResponse for Error {
    fn into_response(mut self) -> Response {
        let mut internal_error = None;

        let (status, error) = if self.is_internal() {
            internal_error = Some(Error {
                kind: self.kind,
                message: self.message.take(),
            });
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiError {
                    message: "An internal server error occured.".into(),
                },
            )
        } else {
            self.to_status_and_api_error()
        };

        let mut response = (status, AppJson(error)).into_response();
        if let Some(error) = internal_error {
            response.extensions_mut().insert(Arc::new(error));
        }
        response
    }
}
