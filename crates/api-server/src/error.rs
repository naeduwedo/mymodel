//! Errors, mapped to OpenAI-shaped bodies with honest status codes.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::openai::{ErrorBody, ErrorDetail};

/// Why a gateway request failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    /// No bearer token, or the wrong one.
    #[error("missing or invalid API token")]
    Unauthorized,

    /// The request body was not a valid chat request.
    #[error("invalid request: {0}")]
    Invalid(String),

    /// The requested model is not available.
    #[error("model not found: {0}")]
    ModelNotFound(String),

    /// The core is not reachable.
    #[error("the core is not reachable: {0}")]
    CoreUnavailable(String),

    /// The core reported a failure.
    #[error("{0}")]
    Backend(String),

    /// No runtime provides this modality on this machine.
    #[error("this capability is not available: {0}")]
    NotAvailable(String),
}

impl ApiError {
    /// The HTTP status this error maps to.
    pub fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::ModelNotFound(_) => StatusCode::NOT_FOUND,
            Self::CoreUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Backend(_) => StatusCode::INTERNAL_SERVER_ERROR,
            // 503, not 500: the server is fine, the capability is missing, and a
            // client should read this as "install or configure a runtime".
            Self::NotAvailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        }
    }

    /// The OpenAI error type string.
    fn kind(&self) -> &'static str {
        match self {
            Self::Unauthorized => "authentication_error",
            Self::Invalid(_) => "invalid_request_error",
            Self::ModelNotFound(_) => "invalid_request_error",
            Self::CoreUnavailable(_) => "api_error",
            Self::Backend(_) => "api_error",
            Self::NotAvailable(_) => "capability_not_available",
        }
    }

    /// The OpenAI error code, when one applies.
    fn code(&self) -> Option<&'static str> {
        match self {
            Self::ModelNotFound(_) => Some("model_not_found"),
            Self::Unauthorized => Some("invalid_api_key"),
            _ => None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorDetail {
                message: self.to_string(),
                kind: self.kind().to_owned(),
                param: None,
                code: self.code().map(str::to_owned),
            },
        };
        (self.status(), Json(body)).into_response()
    }
}

/// A body that is not valid JSON is reported the same way a bad field is, so a
/// client sees one error shape rather than two.
impl From<axum::extract::rejection::JsonRejection> for ApiError {
    fn from(rejection: axum::extract::rejection::JsonRejection) -> Self {
        Self::Invalid(rejection.body_text())
    }
}
