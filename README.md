# eujev — Rust SDK

A typed Rust SDK for the [eu/jev System One API](https://jev.bevel.software/docs),
matching the endpoint and question formats used by the Python and Go clients.
Includes async and optional blocking clients, pooled connections, typed answers,
and structured API errors.

Implemented against the [OpenAPI schema](https://jev.bevel.software/openapi.json)
retrieved on **2026-10-01**. This checkout is ready for local use; it has not been
published to crates.io. Use a current stable Rust toolchain.

## Install locally

In your application's `Cargo.toml`, point `path` to this checkout:

```toml
[dependencies]
eujev = { path = "../rust-sdk-jev" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Quick start

```rust
use eujev::{Client, DecisionRequest, Question};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("EU_JEV_API_KEY")?)?;
    let request = DecisionRequest::new("I was charged twice. Can I get a refund?")
        .with_question("team", Question::choice("Which team should handle this?", [
            ("billing", "Payments, invoices, and refunds"),
            ("support", "Technical issues and bugs"),
        ]));

    let response = client.decide(&request).await?;
    println!("{}", response.answers["team"].choice);
    println!("{} {}", response.meta.request_id, response.meta.cost_eur);
    Ok(())
}
```

The client sends `POST /v1/systemone` with bearer authentication. It takes the
API key explicitly and trims leading/trailing whitespace. Requests use
`jeff-latest` by default; `.with_model("jeff-1.0.0")` pins a release. An empty model
also serializes as the default, without mutating the request.

## Questions and answers

```rust
use eujev::{json, DecisionRequest, Question};

let request = DecisionRequest::new(json!({
    "message": "I was charged twice. Can I get a refund?",
    "customer_tier": "premium"
}))
.with_question("refund", Question::noul_with_criteria("Does this require a refund?", [
    ("true", "A duplicate or incorrect charge"),
    ("false", "A valid charge"),
]))
.with_question("urgency", Question::score("How urgently should the team respond?", [
    "Routine", "Soon", "Immediately",
]));
```

`Question::noul("Is this urgent?")` omits optional criteria. Empty noul criteria
are also omitted. All constructors add the `type` discriminator automatically.
Instructions and criterion descriptions accept `serde_json::Value`: strings,
objects, arrays, and null match the API schema. State accepts a string, object,
or array; use `serde_json::to_value` to convert your own serializable structs.
Question and request fields are public for direct construction or modification.
Adding the same question name again replaces its previous value.

The service validates constraints, including 1–8 questions, 2–10 choice options
or score levels, and a 256 KiB request body limit. This SDK does not duplicate
server validation. `serde_json::Value` represents JSON data; converting nonfinite
floating-point values through `json!`/`Value` can produce null, so validate such
values before conversion if that would be unintended.

Answers are keyed by question name. Compare `answer.question_type` with
`QuestionType::Choice`, `QuestionType::Noul`, or `QuestionType::Score`.

| Question | Answer fields |
| --- | --- |
| Choice | `choice`, `confidence`, `probabilities` |
| Noul | `noul`, a probability from 0 to 1 |
| Score | `score`, an expected position from 0 to N−1; `confidence`, `probabilities`, `legend` |

`noul`, `score`, and `confidence` are `Option<f64>`, preserving valid zero values.
`meta.cost_eur` remains an exact decimal string. `usage` contains input and output
token counts. Metadata includes the request ID, mode, and latency in milliseconds.
The client uses `X-Request-ID` if `meta.request_id` is missing, null, or empty.

Models implement Serde serialization and deserialization. Unknown response
fields are ignored; unknown answer type strings are retained as
`QuestionType::Unknown(String)`. Missing response fields use defaults, and null
scalar/optional fields and null metadata, usage, or answers use defaults. Invalid
types in known fields are rejected; probability and legend map entries must have
numeric and string values respectively. Token counts must be nonnegative integers.
A successful HTTP response must contain a single JSON object.

## HTTP configuration

```rust
use std::time::Duration;
use eujev::Client;

let client = Client::builder("your-api-key")
    .base_url("https://jev.bevel.software") // Service root, without /v1/systemone
    .timeout(Some(Duration::from_secs(20)))
    .build()?;
```

- The default timeout is 60 seconds, including response reading. Override one
  call with `decide_with_timeout(&request, Some(duration))`; `None` disables the
  timeout and a zero duration is rejected.
- Async calls require a Tokio runtime. Dropping their future cancels local I/O;
  work already accepted by the server can still complete and incur a charge.
- Clients are `Clone + Send + Sync`, reuse connections, and support concurrent
  calls. Clones share the underlying pool.
- Base URLs may include a prefix such as `https://example.com/proxy`. Credentials,
  queries, fragments, whitespace, and non-HTTP(S) URLs are rejected.
- Redirects and automatic retries are disabled. All non-2xx responses become
  `Error::Api`, including redirects. The caller decides whether to retry.
- Response bodies are read incrementally and capped at 4 MiB, including error
  responses. Responses are dropped after reading or on error/cancellation.
- HTTPS uses rustls with public web PKI roots. Add a private CA with
  `.add_root_certificate(eujev::Certificate::from_pem(pem_bytes)?)`.
- Reqwest's environment proxy settings are respected. Use `.proxy(...)` with
  `eujev::Proxy` for an explicit proxy, or `.no_proxy()` to disable proxies.
  Call these methods in the desired order: `no_proxy` clears existing proxies;
  a later explicit `proxy` can add one again.

## Blocking client

Enable the optional feature:

```toml
[dependencies]
eujev = { path = "../rust-sdk-jev", features = ["blocking"] }
```

```rust
use eujev::{blocking::Client, DecisionRequest, Question};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::new(std::env::var("EU_JEV_API_KEY")?)?;
    let request = DecisionRequest::new("I was charged twice.")
        .with_question("refund", Question::noul("Does this require a refund?"));
    let response = client.decide(&request)?;
    println!("{:?}", response.answers["refund"].noul);
    Ok(())
}
```

The blocking client provides the same builders, timeouts, and response/error
models. Construct, use, and drop it outside async runtimes. Within Tokio, use the
async client or wrap the entire blocking client lifetime in `spawn_blocking`.

## Errors

```rust
use eujev::Error;

match client.decide(&request).await {
    Ok(response) => println!("{}", response.meta.request_id),
    Err(Error::Api(error)) => {
        eprintln!("{} {}: {}", error.status, error.code, error.message);
        eprintln!("request_id={} retry_after={}", error.request_id, error.retry_after);
        if let Some(cause) = &error.body_error {
            eprintln!("Could not read the complete error body: {cause}");
        }
    }
    Err(error) if error.is_timeout() => eprintln!("Request timed out"),
    Err(error) => eprintln!("{error}"),
}
```

`ApiError` includes `status`, `message`, `code`, `contact_url`, `request_id`,
`retry_after`, case-insensitive `headers`, raw `body` bytes, `body_truncated`, and
an optional `body_error`. `body_text()` returns lossy UTF-8 for display. Non-JSON
error bodies retain status and raw bytes. `Retry-After` is preserved verbatim.

`Error` distinguishes configuration, client construction, encoding, transport,
response body, JSON decoding, and HTTP failures. `ResponseBodyError::TooLarge`
identifies oversized responses. Underlying failures remain available via
`std::error::Error::source`; `error.is_timeout()` also checks error response reads.
`error.as_api_error()` provides borrowed access to HTTP error details.

## Examples and development

Run the offline encoding/parsing example without an API key or network:

```sh
cargo run --example offline
```

The following examples each make a live request subject to service billing:

```sh
export EU_JEV_API_KEY='your-api-key'
cargo run --example refund
cargo run --example all_question_types
cargo run --example handle_errors
cargo run --example blocking --features blocking
```

Development checks:

```sh
cargo fmt --all -- --check
cargo clippy --all-features --all-targets -- -D warnings
cargo test
cargo test --all-features
cargo doc --all-features --no-deps
cargo package --all-features
```

Tests use local HTTP servers; no API key or live service requests are needed.
They cover wire formats, response parsing, authentication, errors, redirects,
timeouts, cancellation, concurrency, truncated responses, and response limits
for async and blocking clients. CI runs on Linux, macOS, and Windows.
