use std::{borrow::Cow, fmt};

use reqwest::{header::HeaderMap, StatusCode};
use serde::Deserialize;

use crate::models::null_default;

/// Result type returned by SDK operations.
pub type Result<T> = std::result::Result<T, Error>;

/// A client configuration, encoding, HTTP, or response failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Configuration was rejected before sending a request.
    #[error("eujev: invalid configuration: {0}")]
    Configuration(&'static str),
    /// The underlying HTTP client could not be constructed.
    #[error("eujev: build HTTP client: {0}")]
    ClientBuild(#[source] reqwest::Error),
    /// The request could not be encoded; nothing was sent.
    #[error("eujev: encode request: {0}")]
    Encode(#[source] serde_json::Error),
    /// Sending the HTTP request failed, including a timeout before headers.
    #[error("eujev: send request: {0}")]
    Transport(#[source] reqwest::Error),
    /// Reading a successful response failed or exceeded the size limit.
    #[error("eujev: {0}")]
    ResponseBody(#[from] ResponseBodyError),
    /// A successful response was not a valid typed JSON object.
    #[error("eujev: decode response: {0}")]
    Decode(#[source] serde_json::Error),
    /// Any non-2xx response, including redirects and non-JSON proxy errors.
    #[error(transparent)]
    Api(#[from] Box<ApiError>),
}

impl Error {
    /// Return structured HTTP error details, if an HTTP failure was received.
    pub fn as_api_error(&self) -> Option<&ApiError> {
        match self {
            Self::Api(error) => Some(error),
            _ => None,
        }
    }

    /// Whether a transport or body read timed out, including on error responses.
    pub fn is_timeout(&self) -> bool {
        match self {
            Self::Transport(error) => error.is_timeout(),
            Self::ResponseBody(error) => error.is_timeout(),
            Self::Api(error) => error
                .body_error
                .as_ref()
                .is_some_and(ResponseBodyError::is_timeout),
            _ => false,
        }
    }
}

/// A response body read failure. API errors retain this without losing status.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResponseBodyError {
    /// More than [`crate::MAX_RESPONSE_BYTES`] were received.
    #[error("response body exceeds 4 MiB")]
    TooLarge,
    /// The async response stream failed.
    #[error("read response: {0}")]
    Read(#[source] reqwest::Error),
    /// The blocking response stream failed.
    #[cfg(feature = "blocking")]
    #[error("read response: {0}")]
    Io(#[source] std::io::Error),
}

impl ResponseBodyError {
    /// Whether the underlying read timed out.
    pub fn is_timeout(&self) -> bool {
        match self {
            Self::Read(error) => error.is_timeout(),
            #[cfg(feature = "blocking")]
            Self::Io(error) => {
                use std::error::Error as _;
                if error.kind() == std::io::ErrorKind::TimedOut {
                    return true;
                }
                let mut cause = error.source();
                while let Some(source) = cause {
                    if let Some(error) = source.downcast_ref::<reqwest::Error>() {
                        if error.is_timeout() {
                            return true;
                        }
                    }
                    cause = source.source();
                }
                false
            }
            _ => false,
        }
    }
}

/// Details of a non-2xx HTTP response.
#[derive(Debug)]
pub struct ApiError {
    /// HTTP status, even if the body could not be decoded or fully read.
    pub status: StatusCode,
    /// Human-readable service error, or the standard HTTP reason phrase.
    pub message: String,
    /// Optional machine-readable code, such as `rate_limited`.
    pub code: String,
    /// Service-provided contact URL, if present.
    pub contact_url: String,
    /// The `X-Request-ID` header, if present.
    pub request_id: String,
    /// The `Retry-After` header retained verbatim for caller-controlled retries.
    pub retry_after: String,
    /// Case-insensitive response headers, including repeated values.
    pub headers: HeaderMap,
    /// Raw response bytes, capped at [`crate::MAX_RESPONSE_BYTES`].
    pub body: Vec<u8>,
    /// True if the body was cut off at the SDK size limit.
    pub body_truncated: bool,
    /// A size or read failure encountered while reading this error response.
    pub body_error: Option<ResponseBodyError>,
}

impl ApiError {
    pub(crate) fn new(
        status: StatusCode,
        headers: HeaderMap,
        body: Vec<u8>,
        body_error: Option<ResponseBodyError>,
    ) -> Self {
        #[derive(Default, Deserialize)]
        #[serde(default)]
        struct Payload {
            #[serde(deserialize_with = "null_default")]
            error: String,
            #[serde(deserialize_with = "null_default")]
            code: String,
            #[serde(deserialize_with = "null_default")]
            contact_url: String,
        }
        let payload: Payload =
            serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&body)
                .and_then(|map| serde_json::from_value(serde_json::Value::Object(map)))
                .unwrap_or_default();
        Self {
            status,
            message: if payload.error.is_empty() {
                status.canonical_reason().unwrap_or_default().into()
            } else {
                payload.error
            },
            code: payload.code,
            contact_url: payload.contact_url,
            request_id: header_text(&headers, "x-request-id"),
            retry_after: header_text(&headers, "retry-after"),
            headers,
            body,
            body_truncated: matches!(body_error, Some(ResponseBodyError::TooLarge)),
            body_error,
        }
    }

    /// Raw body decoded as UTF-8, replacing invalid sequences.
    pub fn body_text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.body)
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "eujev: HTTP {}", self.status.as_u16())?;
        if !self.code.is_empty() {
            write!(f, " ({})", self.code)?;
        }
        if !self.message.is_empty() {
            write!(f, ": {}", self.message)?;
        }
        if !self.request_id.is_empty() {
            write!(f, " [request_id={}]", self.request_id)?;
        }
        Ok(())
    }
}

impl std::error::Error for ApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.body_error.as_ref().map(|error| error as _)
    }
}

pub(crate) fn header_text(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .into()
}
