//! Synchronous client, available with the `blocking` feature.
//!
//! Use this client outside async runtimes. Inside Tokio, prefer [`crate::Client`]
//! or move the entire blocking operation (including construction and drop) into
//! `tokio::task::spawn_blocking`.

use std::{fmt, io::Read, time::Duration};

use reqwest::{
    header::{self, HeaderValue},
    Certificate, Proxy, Url,
};

use crate::{
    client::{finish_response, validate_timeout, Configuration},
    DecisionRequest, DecisionResponse, Error, ResponseBodyError, Result, MAX_RESPONSE_BYTES,
    USER_AGENT,
};

/// Synchronous client with pooled connections. Clones can be shared across threads.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::blocking::Client,
    endpoint: Url,
    authorization: HeaderValue,
    timeout: Option<Duration>,
}

impl Client {
    /// Create a client with default HTTP settings.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::builder(api_key).build()
    }

    /// Configure a synchronous client before construction.
    pub fn builder(api_key: impl Into<String>) -> ClientBuilder {
        ClientBuilder {
            config: Configuration::new(api_key),
        }
    }

    /// Send a decision without mutating the request.
    pub fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse> {
        self.send(request, self.timeout)
    }

    /// Override the overall timeout for this call. `None` disables the deadline.
    pub fn decide_with_timeout(
        &self,
        request: &DecisionRequest,
        timeout: Option<Duration>,
    ) -> Result<DecisionResponse> {
        validate_timeout(timeout)?;
        self.send(request, timeout)
    }

    fn send(
        &self,
        request: &DecisionRequest,
        timeout: Option<Duration>,
    ) -> Result<DecisionResponse> {
        let payload = serde_json::to_vec(request).map_err(Error::Encode)?;
        let mut builder = self
            .http
            .post(self.endpoint.clone())
            .header(header::AUTHORIZATION, self.authorization.clone())
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json")
            .body(payload);
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        let mut response = builder.send().map_err(Error::Transport)?;
        let status = response.status();
        let headers = response.headers().clone();
        let mut body = Vec::new();
        let mut buffer = [0; 16 * 1024];
        let failure = loop {
            let remaining = (MAX_RESPONSE_BYTES + 1 - body.len()).min(buffer.len());
            match response.read(&mut buffer[..remaining]) {
                Ok(0) => break None,
                Ok(count) => {
                    body.extend_from_slice(&buffer[..count]);
                    if body.len() > MAX_RESPONSE_BYTES {
                        body.truncate(MAX_RESPONSE_BYTES);
                        break Some(ResponseBodyError::TooLarge);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => break Some(ResponseBodyError::Io(error)),
            }
        };
        finish_response(status, headers, body, failure)
    }
}

/// Builder for the blocking client. Secrets are redacted in debug output.
pub struct ClientBuilder {
    config: Configuration,
}

impl ClientBuilder {
    /// Set the service root, optionally including a proxy path prefix.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.config.base_url = url.into();
        self
    }

    /// Set the overall request timeout. `None` disables it; zero is invalid.
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Add a custom TLS root certificate alongside the built-in roots.
    pub fn add_root_certificate(mut self, certificate: Certificate) -> Self {
        self.config.certificates.push(certificate);
        self
    }

    /// Add an explicit proxy, overriding automatic environment proxy selection.
    pub fn proxy(mut self, proxy: Proxy) -> Self {
        self.config.proxies.push(proxy);
        self
    }

    /// Disable automatic and explicitly configured proxies.
    pub fn no_proxy(mut self) -> Self {
        self.config.no_proxy = true;
        self.config.proxies.clear();
        self
    }

    /// Validate configuration and build the synchronous connection pool.
    pub fn build(self) -> Result<Client> {
        let (endpoint, authorization) = self.config.validate()?;
        let mut builder = reqwest::blocking::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            // reqwest's blocking default is 30 seconds; apply our deadline per request.
            .timeout(None);
        if self.config.no_proxy {
            builder = builder.no_proxy();
        }
        for proxy in self.config.proxies {
            builder = builder.proxy(proxy);
        }
        for certificate in self.config.certificates {
            builder = builder.add_root_certificate(certificate);
        }
        Ok(Client {
            http: builder.build().map_err(Error::ClientBuild)?,
            endpoint,
            authorization,
            timeout: self.config.timeout,
        })
    }
}

impl fmt::Debug for ClientBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.config.fmt(f)
    }
}
