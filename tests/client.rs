mod support;

use eujev::{
    json, Client, DecisionRequest, Error, Question, ResponseBodyError, MAX_RESPONSE_BYTES,
};
use std::{error::Error as _, time::Duration};
use support::{Reply, Server};

const CHOICE_RESPONSE: &str = r#"{
    "model":"jeff-1.0.0",
    "answers":{"team":{"type":"choice","choice":"billing","confidence":0.9,"probabilities":{"billing":0.95,"support":0.05}}},
    "usage":{"input_tokens":64,"output_tokens":0},
    "meta":{"request_id":"body-id","mode":"live","latency_ms":180,"cost_eur":"0.000002368"}
}"#;

fn request() -> DecisionRequest {
    DecisionRequest::new("I was charged twice. Can I get a refund?").with_question(
        "team",
        Question::choice(
            "Which team should handle this?",
            [
                ("billing", "Payments, invoices, and refunds"),
                ("support", "Technical issues and bugs"),
            ],
        ),
    )
}

fn client(server: &Server) -> Client {
    Client::builder("test-key")
        .base_url(&server.url)
        .no_proxy()
        .build()
        .unwrap()
}

#[tokio::test]
async fn request_matches_api_and_preserves_response_fields() {
    let mut server = Server::start(Reply::new(
        200,
        &[("X-Request-ID", "header-id")],
        CHOICE_RESPONSE,
    ))
    .await;
    let client = Client::builder("  secret-test-key \n")
        .base_url(format!("{}/proxy///", server.url))
        .no_proxy()
        .build()
        .unwrap();
    let request = request().with_model("");
    let result = client.decide(&request).await.unwrap();
    let received = server.request().await;
    assert_eq!(received.line, "POST /proxy/v1/systemone HTTP/1.1");
    assert_eq!(received.headers["authorization"], "Bearer secret-test-key");
    assert_eq!(received.headers["content-type"], "application/json");
    assert_eq!(received.headers["accept"], "application/json");
    assert_eq!(
        received.headers["user-agent"],
        concat!("eujev/", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&received.body).unwrap(),
        json!({
            "model": "jeff-latest", "state": "I was charged twice. Can I get a refund?",
            "questions": {"team": {"type": "choice", "instructions": "Which team should handle this?", "criteria": {"billing": "Payments, invoices, and refunds", "support": "Technical issues and bugs"}}}
        })
    );
    assert!(request.model.is_empty());
    assert_eq!(result.answers["team"].choice, "billing");
    assert_eq!(result.answers["team"].confidence, Some(0.9));
    assert_eq!(result.usage.input_tokens, 64);
    assert_eq!(result.meta.cost_eur, "0.000002368");
    assert_eq!(result.meta.request_id, "body-id");
    assert_eq!(server.calls(), 1);
    assert!(!format!("{client:?}").contains("secret-test-key"));
}

#[tokio::test]
async fn request_id_falls_back_to_case_insensitive_header() {
    for body in ["{}", r#"{"meta":{"request_id":""}}"#, r#"{"meta":null}"#] {
        let server = Server::start(Reply::new(201, &[("x-ReQuEsT-iD", "fallback")], body)).await;
        let response = client(&server).decide(&request()).await.unwrap();
        assert_eq!(response.meta.request_id, "fallback");
    }
}

#[tokio::test]
async fn api_errors_keep_status_headers_body_and_details_without_retries() {
    let body = r#"{"error":"Add credit","code":"insufficient_credits","contact_url":"https://jev.bevel.software"}"#;
    for status in [400, 401, 402, 413, 415, 422, 429, 503] {
        let server = Server::start(Reply::new(
            status,
            &[
                ("Retry-After", "30"),
                ("X-Request-ID", "err-id"),
                ("X-Extra", "one"),
                ("X-Extra", "two"),
            ],
            body,
        ))
        .await;
        let failure = client(&server).decide(&request()).await.unwrap_err();
        let error = failure.as_api_error().unwrap();
        assert_eq!(error.status.as_u16(), status);
        assert_eq!(error.message, "Add credit");
        assert_eq!(error.code, "insufficient_credits");
        assert_eq!(error.contact_url, "https://jev.bevel.software");
        assert_eq!(error.request_id, "err-id");
        assert_eq!(error.retry_after, "30");
        assert_eq!(error.headers.get_all("x-extra").iter().count(), 2);
        assert_eq!(error.body, body.as_bytes());
        assert!(!error.body_truncated);
        assert!(error.body_error.is_none());
        assert!(error.to_string().contains("insufficient_credits"));
        assert_eq!(server.calls(), 1);
    }
}

#[tokio::test]
async fn non_json_and_malformed_api_errors_keep_raw_bytes() {
    for body in [
        b"<html>bad gateway</html>".as_slice(),
        b"\xff\xfe",
        br#"{"error":7,"code":"bad"}"#,
        b"null",
        br#"["not-an-error-object","bad","https://example.com"]"#,
    ] {
        let server = Server::start(Reply::new(502, &[], body)).await;
        let failure = client(&server).decide(&request()).await.unwrap_err();
        let error = failure.as_api_error().unwrap();
        assert_eq!(error.message, "Bad Gateway");
        assert_eq!(error.body, body);
        assert!(error.code.is_empty());
        assert_eq!(error.body_text(), String::from_utf8_lossy(body));
    }
}

#[tokio::test]
async fn redirects_are_returned_without_following_location() {
    let target = Server::start(Reply::json(CHOICE_RESPONSE)).await;
    for status in [301, 302, 303, 307, 308] {
        let source =
            Server::start(Reply::new(status, &[("Location", &target.url)], "redirect")).await;
        let error = client(&source).decide(&request()).await.unwrap_err();
        assert_eq!(error.as_api_error().unwrap().status.as_u16(), status);
        assert_eq!(source.calls(), 1);
        assert_eq!(target.calls(), 0);
    }
}

#[tokio::test]
async fn malformed_success_is_a_decode_error() {
    for body in [
        "",
        "null",
        "[]",
        "{} {}",
        "not json",
        r#"{"answers":{"q":{"noul":"no"}}}"#,
    ] {
        let server = Server::start(Reply::json(body)).await;
        let error = client(&server).decide(&request()).await.unwrap_err();
        assert!(matches!(error, Error::Decode(_)), "{error:?}");
        assert!(error.source().is_some());
    }
}

#[tokio::test]
async fn response_size_limit_handles_exact_boundary_and_chunked_bodies() {
    let mut exact = b"{}".to_vec();
    exact.resize(MAX_RESPONSE_BYTES, b' ');
    let server = Server::start(Reply::new(200, &[], &exact)).await;
    client(&server).decide(&request()).await.unwrap();
    exact.push(b' ');
    for status in [200, 503] {
        let mut reply = Reply::new(status, &[], &exact);
        // Exercise the streaming limit without a trustworthy Content-Length.
        reply.head = format!(
            "HTTP/1.1 {status} Test\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
        )
        .into_bytes();
        reply.body = format!("{:x}\r\n", exact.len()).into_bytes();
        reply.body.extend_from_slice(&exact);
        reply.body.extend_from_slice(b"\r\n0\r\n\r\n");
        let server = Server::start(reply).await;
        let failure = client(&server).decide(&request()).await.unwrap_err();
        if status == 200 {
            assert!(matches!(
                failure,
                Error::ResponseBody(ResponseBodyError::TooLarge)
            ));
        } else {
            let error = failure.as_api_error().unwrap();
            assert_eq!(error.status.as_u16(), 503);
            assert_eq!(error.body.len(), MAX_RESPONSE_BYTES);
            assert!(error.body_truncated);
            assert!(matches!(
                error.body_error,
                Some(ResponseBodyError::TooLarge)
            ));
            assert!(error.source().is_some());
        }
    }
}

#[tokio::test]
async fn incomplete_responses_preserve_http_error_status() {
    for status in [200, 503] {
        let mut reply = Reply::new(status, &[], r#"{"error":"partial"}"#);
        reply.head =
            format!("HTTP/1.1 {status} Test\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n")
                .into_bytes();
        let server = Server::start(reply).await;
        let error = client(&server).decide(&request()).await.unwrap_err();
        if status == 200 {
            assert!(matches!(
                error,
                Error::ResponseBody(ResponseBodyError::Read(_))
            ));
        } else {
            let error = error.as_api_error().unwrap();
            assert_eq!(error.status.as_u16(), 503);
            assert!(matches!(error.body_error, Some(ResponseBodyError::Read(_))));
            assert!(!error.body_truncated);
        }
    }
}

#[tokio::test]
async fn timeout_covers_headers_and_body_and_can_be_overridden() {
    for (status, slow_headers) in [(200, true), (200, false), (503, false)] {
        let mut reply = Reply::new(status, &[], "{}");
        if slow_headers {
            reply.before_headers = Duration::from_millis(300);
        } else {
            reply.before_body = Duration::from_millis(300);
        }
        let server = Server::start(reply).await;
        let client = Client::builder("key")
            .base_url(&server.url)
            .no_proxy()
            .timeout(Some(Duration::from_millis(50)))
            .build()
            .unwrap();
        let error = client.decide(&request()).await.unwrap_err();
        assert!(error.is_timeout(), "{error:?}");
        if status == 503 {
            assert_eq!(error.as_api_error().unwrap().status.as_u16(), 503);
        } else {
            client.decide_with_timeout(&request(), None).await.unwrap();
        }
    }
}

#[tokio::test]
async fn cancellation_and_connection_failures_are_exposed() {
    let mut reply = Reply::json("{}");
    reply.before_headers = Duration::from_secs(2);
    let server = Server::start(reply).await;
    assert!(tokio::time::timeout(
        Duration::from_millis(50),
        client(&server).decide(&request())
    )
    .await
    .is_err());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = Client::builder("key")
        .base_url(format!("http://{address}"))
        .no_proxy()
        .build()
        .unwrap();
    let error = client.decide(&request()).await.unwrap_err();
    assert!(matches!(error, Error::Transport(_)));
    assert!(error.source().is_some());
}

#[tokio::test]
async fn cloned_client_supports_concurrent_calls() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Client>();
    let server = Server::start(Reply::json(CHOICE_RESPONSE)).await;
    let client = client(&server);
    let mut calls = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let client = client.clone();
        calls.spawn(async move { client.decide(&request()).await.unwrap() });
    }
    while let Some(result) = calls.join_next().await {
        assert_eq!(result.unwrap().answers["team"].choice, "billing");
    }
    assert_eq!(server.calls(), 8);
}

#[test]
fn invalid_configuration_is_rejected_without_leaking_secrets() {
    for key in ["", "  ", "two words", "a\nb", "é", "a\u{7f}"] {
        assert!(matches!(Client::new(key), Err(Error::Configuration(_))));
    }
    for url in [
        "",
        "localhost",
        "ftp://example.com",
        "https:///example.com",
        "https:example.com",
        "https://",
        "https://a:bad",
        "https://a:65536",
        "https://a?",
        "https://a#",
        "https://a?x=1",
        "https://a#f",
        "https://u:p@example.com",
        "https://@example.com",
        "https://a/with space",
        "https://a\\evil",
        "https://a\n",
    ] {
        let result = Client::builder("secret").base_url(url).build();
        assert!(
            matches!(result, Err(Error::Configuration(_))),
            "accepted {url}: {result:?}"
        );
    }
    assert!(Client::builder("key")
        .timeout(Some(Duration::ZERO))
        .build()
        .is_err());
    let builder =
        Client::builder("do-not-print-key").base_url("https://secret-password@example.com");
    assert!(!format!("{builder:?}").contains("do-not-print-key"));
    assert!(!format!("{builder:?}").contains("secret-password"));
    assert!(!builder
        .build()
        .unwrap_err()
        .to_string()
        .contains("secret-password"));
}

#[cfg(feature = "blocking")]
#[tokio::test]
async fn blocking_client_uses_same_wire_format_and_error_handling() {
    let mut server = Server::start(Reply::new(200, &[("X-Request-ID", "blocking-id")], "{}")).await;
    let url = format!("{}/prefix///", server.url);
    let result = tokio::task::spawn_blocking(move || {
        let client = eujev::blocking::Client::builder("block-key")
            .base_url(url)
            .no_proxy()
            .build()
            .unwrap();
        assert!(!format!("{client:?}").contains("block-key"));
        client.decide(&request()).unwrap()
    })
    .await
    .unwrap();
    assert_eq!(result.meta.request_id, "blocking-id");
    let received = server.request().await;
    assert_eq!(received.line, "POST /prefix/v1/systemone HTTP/1.1");
    assert_eq!(received.headers["authorization"], "Bearer block-key");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&received.body).unwrap(),
        serde_json::to_value(request()).unwrap()
    );

    let target = Server::start(Reply::json("{}")).await;
    let redirect = Server::start(Reply::new(307, &[("Location", &target.url)], "{}")).await;
    let url = redirect.url.clone();
    let error = tokio::task::spawn_blocking(move || {
        eujev::blocking::Client::builder("key")
            .base_url(url)
            .no_proxy()
            .build()
            .unwrap()
            .decide(&request())
            .unwrap_err()
    })
    .await
    .unwrap();
    assert_eq!(error.as_api_error().unwrap().status.as_u16(), 307);
    assert_eq!(target.calls(), 0);
}

#[cfg(feature = "blocking")]
#[tokio::test]
async fn blocking_timeouts_and_body_size_errors() {
    for status in [200, 503] {
        let server =
            Server::start(Reply::new(status, &[], vec![b'x'; MAX_RESPONSE_BYTES + 1])).await;
        let url = server.url.clone();
        let error = tokio::task::spawn_blocking(move || {
            eujev::blocking::Client::builder("key")
                .base_url(url)
                .no_proxy()
                .build()
                .unwrap()
                .decide(&request())
                .unwrap_err()
        })
        .await
        .unwrap();
        if status == 200 {
            assert!(matches!(
                error,
                Error::ResponseBody(ResponseBodyError::TooLarge)
            ));
        } else {
            let error = error.as_api_error().unwrap();
            assert_eq!(error.body.len(), MAX_RESPONSE_BYTES);
            assert!(error.body_truncated);
        }
    }
    for status in [200, 503] {
        let mut reply = Reply::new(status, &[], "{}");
        reply.before_body = Duration::from_millis(300);
        let server = Server::start(reply).await;
        let url = server.url.clone();
        let error = tokio::task::spawn_blocking(move || {
            let client = eujev::blocking::Client::builder("key")
                .base_url(url)
                .no_proxy()
                .build()
                .unwrap();
            client
                .decide_with_timeout(&request(), Some(Duration::from_millis(50)))
                .unwrap_err()
        })
        .await
        .unwrap();
        assert!(error.is_timeout(), "{error:?}");
        if status == 503 {
            assert_eq!(error.as_api_error().unwrap().status.as_u16(), 503);
        }
    }
}
