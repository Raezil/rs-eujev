//! A typed Rust client for the [eu/jev System One API](https://jev.bevel.software/docs).
//!
//! ```no_run
//! use eujev::{Client, DecisionRequest, Question};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = Client::new(std::env::var("EU_JEV_API_KEY")?)?;
//! let request = DecisionRequest::new("I was charged twice. Can I get a refund?")
//!     .with_question("team", Question::choice("Which team should handle this?", [
//!         ("billing", "Payments, invoices, and refunds"),
//!         ("support", "Technical issues and bugs"),
//!     ]));
//! let response = client.decide(&request).await?;
//! println!("{}", response.answers["team"].choice);
//! # Ok(())
//! # }
//! ```
//!
//! The async client requires a Tokio runtime. Enable the `blocking` feature for
//! `blocking::Client`. Clients reuse connections and can be cloned and shared.
//! Redirects and automatic retries are disabled; the service validates input
//! constraints. Costs remain strings to retain their exact decimal value.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod client;
mod error;
mod models;

#[cfg(feature = "blocking")]
pub mod blocking;

pub use client::{Client, ClientBuilder};
pub use error::{ApiError, Error, ResponseBodyError, Result};
pub use models::{
    Answer, DecisionRequest, DecisionResponse, Metadata, Question, QuestionType, Usage,
};
pub use reqwest::{header, Certificate, Proxy, StatusCode};
pub use serde_json::{json, Value};

/// Default service root URL.
pub const DEFAULT_BASE_URL: &str = "https://jev.bevel.software";
/// Model selected when no model is specified or its name is empty.
pub const DEFAULT_MODEL: &str = "jeff-latest";
/// Default overall HTTP timeout, including reading the response body.
pub const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Maximum number of response bytes retained by the SDK (4 MiB).
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"));
