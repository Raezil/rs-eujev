use std::{fmt, time::Duration};

use reqwest::{
    header::{self, HeaderMap, HeaderValue},
    Certificate, Proxy, StatusCode, Url,
};

use crate::{
    error::header_text, ApiError, DecisionRequest, DecisionResponse, Error, ResponseBodyError,
    Result, DEFAULT_BASE_URL, DEFAULT_TIMEOUT, MAX_RESPONSE_BYTES, USER_AGENT,
};

/// Async client with pooled connections. Clones share the connection pool.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    endpoint: Url,
    authorization: HeaderValue,
    timeout: Option<Duration>,
}

impl Client {
    /// Create a client with default HTTP settings. Pass the token without `Bearer`.
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        Self::builder(api_key).build()
    }

    /// Configure a client before construction.
    pub fn builder(api_key: impl Into<String>) -> ClientBuilder {
        ClientBuilder::new(api_key)
    }

    /// Send a decision without mutating the request. Dropping this future cancels
    /// local I/O; it cannot undo work already accepted by the service.
    pub async fn decide(&self, request: &DecisionRequest) -> Result<DecisionResponse> {
        self.send(request, self.timeout).await
    }

    /// Override the overall timeout for one call. `None` disables the deadline.
    pub async fn decide_with_timeout(
        &self,
        request: &DecisionRequest,
        timeout: Option<Duration>,
    ) -> Result<DecisionResponse> {
        validate_timeout(timeout)?;
        self.send(request, timeout).await
    }

    async fn send(
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
        let mut response = builder.send().await.map_err(Error::Transport)?;
        let status = response.status();
        let headers = response.headers().clone();
        let mut body = Vec::new();
        let failure = loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    let remaining = MAX_RESPONSE_BYTES - body.len();
                    body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
                    if chunk.len() > remaining {
                        break Some(ResponseBodyError::TooLarge);
                    }
                }
                Ok(None) => break None,
                Err(error) => break Some(ResponseBodyError::Read(error)),
            }
        };
        finish_response(status, headers, body, failure)
    }
}

/// HTTP client configuration. Secrets are redacted in debug output.
pub struct ClientBuilder {
    pub(crate) config: Configuration,
}

impl ClientBuilder {
    fn new(api_key: impl Into<String>) -> Self {
        Self {
            config: Configuration::new(api_key),
        }
    }

    /// Set the service root. A proxy path prefix is retained.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.config.base_url = url.into();
        self
    }

    /// Set the overall request deadline. `None` disables it; zero is invalid.
    pub fn timeout(mut self, timeout: Option<Duration>) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Add a custom TLS root certificate alongside the built-in public roots.
    pub fn add_root_certificate(mut self, certificate: Certificate) -> Self {
        self.config.certificates.push(certificate);
        self
    }

    /// Add an explicit proxy. This overrides automatic environment proxy selection.
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

    /// Validate configuration and construct the connection pool.
    pub fn build(self) -> Result<Client> {
        let (endpoint, authorization) = self.config.validate()?;
        let mut builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never());
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

pub(crate) struct Configuration {
    api_key: String,
    pub(crate) base_url: String,
    pub(crate) timeout: Option<Duration>,
    pub(crate) certificates: Vec<Certificate>,
    pub(crate) proxies: Vec<Proxy>,
    pub(crate) no_proxy: bool,
}

impl Configuration {
    pub(crate) fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.into(),
            timeout: Some(DEFAULT_TIMEOUT),
            certificates: Vec::new(),
            proxies: Vec::new(),
            no_proxy: false,
        }
    }

    pub(crate) fn validate(&self) -> Result<(Url, HeaderValue)> {
        validate_timeout(self.timeout)?;
        let key = self.api_key.trim();
        if key.is_empty() || !key.bytes().all(|b| (33..=126).contains(&b)) {
            return Err(Error::Configuration(
                "API key must be a nonempty ASCII token without whitespace or controls",
            ));
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {key}"))
            .map_err(|_| Error::Configuration("invalid API key"))?;
        authorization.set_sensitive(true);
        let invalid_url = || {
            Error::Configuration("base URL must be an HTTP(S) URL without credentials, whitespace, a query, or a fragment")
        };
        if self
            .base_url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '\\' | '?' | '#'))
        {
            return Err(invalid_url());
        }
        // Validate the raw authority too: URL parsing normalizes empty userinfo
        // and permissive forms like https:example.com.
        let (_, rest) = self.base_url.split_once("://").ok_or_else(invalid_url)?;
        let authority = rest.split('/').next().unwrap_or_default();
        if authority.is_empty() || authority.contains('@') {
            return Err(invalid_url());
        }
        let url = Url::parse(&self.base_url).map_err(|_| invalid_url())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid_url());
        }
        let endpoint = Url::parse(&format!(
            "{}/v1/systemone",
            url.as_str().trim_end_matches('/')
        ))
        .map_err(|_| invalid_url())?;
        Ok((endpoint, authorization))
    }
}

impl fmt::Debug for Configuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Do not print unvalidated base URLs or proxies, which might contain credentials.
        f.debug_struct("ClientBuilder")
            .field("api_key", &"[REDACTED]")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

pub(crate) fn validate_timeout(timeout: Option<Duration>) -> Result<()> {
    if timeout.is_some_and(|timeout| timeout.is_zero()) {
        return Err(Error::Configuration("timeout must be positive, or None"));
    }
    Ok(())
}

pub(crate) fn finish_response(
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
    failure: Option<ResponseBodyError>,
) -> Result<DecisionResponse> {
    if !status.is_success() {
        return Err(Error::Api(Box::new(ApiError::new(
            status, headers, body, failure,
        ))));
    }
    if let Some(error) = failure {
        return Err(error.into());
    }
    let mut response: DecisionResponse = serde_json::from_slice(&body).map_err(Error::Decode)?;
    if response.meta.request_id.is_empty() {
        response.meta.request_id = header_text(&headers, "x-request-id");
    }
    Ok(response)
}
