use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::{JoinHandle, JoinSet},
};

#[derive(Clone)]
pub struct Reply {
    pub head: Vec<u8>,
    pub body: Vec<u8>,
    pub before_headers: Duration,
    pub before_body: Duration,
}

impl Reply {
    pub fn new(status: u16, headers: &[(&str, &str)], body: impl AsRef<[u8]>) -> Self {
        let body = body.as_ref().to_vec();
        let mut head = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        head.push_str("\r\n");
        Self {
            head: head.into_bytes(),
            body,
            before_headers: Duration::ZERO,
            before_body: Duration::ZERO,
        }
    }

    pub fn json(body: &str) -> Self {
        Self::new(200, &[("Content-Type", "application/json")], body)
    }
}

#[derive(Debug)]
pub struct CapturedRequest {
    pub line: String,
    pub headers: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

pub struct Server {
    pub url: String,
    requests: mpsc::Receiver<CapturedRequest>,
    calls: Arc<AtomicUsize>,
    task: JoinHandle<()>,
}

impl Server {
    pub async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::channel(64);
        let calls = Arc::new(AtomicUsize::new(0));
        let count = calls.clone();
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut socket, _) = accepted.unwrap();
                        count.fetch_add(1, Ordering::SeqCst);
                        let sender = sender.clone();
                        let reply = reply.clone();
                        connections.spawn(async move {
                            let received = tokio::time::timeout(Duration::from_secs(5), async {
                                let mut bytes = Vec::new();
                                let mut buffer = [0; 4096];
                                let end = loop {
                                    let count = socket.read(&mut buffer).await.ok()?;
                                    if count == 0 { return None; }
                                    bytes.extend_from_slice(&buffer[..count]);
                                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") { break end; }
                                    assert!(bytes.len() < 64 * 1024, "unexpected request headers");
                                };
                                let header_text = std::str::from_utf8(&bytes[..end]).ok()?;
                                let mut lines = header_text.split("\r\n");
                                let line = lines.next()?.to_string();
                                let headers: BTreeMap<String, String> = lines.map(|line| {
                                    let (name, value) = line.split_once(':').unwrap();
                                    (name.to_ascii_lowercase(), value.trim().to_string())
                                }).collect();
                                let length: usize = headers.get("content-length")?.parse().ok()?;
                                let mut body = bytes[end + 4..].to_vec();
                                while body.len() < length {
                                    let count = socket.read(&mut buffer).await.ok()?;
                                    if count == 0 { return None; }
                                    body.extend_from_slice(&buffer[..count]);
                                }
                                Some(CapturedRequest { line, headers, body })
                            }).await.unwrap();
                            let Some(request) = received else { return; };
                            sender.send(request).await.unwrap();
                            tokio::time::sleep(reply.before_headers).await;
                            if socket.write_all(&reply.head).await.is_err() { return; }
                            tokio::time::sleep(reply.before_body).await;
                            let _ = socket.write_all(&reply.body).await;
                            let _ = socket.shutdown().await;
                        });
                    }
                    Some(result) = connections.join_next(), if !connections.is_empty() => { result.unwrap(); }
                }
            }
        });
        Self {
            url,
            requests,
            calls,
            task,
        }
    }

    pub async fn request(&mut self) -> CapturedRequest {
        tokio::time::timeout(Duration::from_secs(5), self.requests.recv())
            .await
            .unwrap()
            .unwrap()
    }

    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
