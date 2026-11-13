//! Test-only HTTP sink for the webhook smoke test: `cargo run --example http_sink -- 127.0.0.1:3494 /tmp/capture`.
//! Accepts connections until killed, appends every raw request to the capture file and answers 200 OK.
use std::io::Write;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let addr = args.get(1).map(String::as_str).unwrap_or("127.0.0.1:3494");
    let capture = args.get(2).cloned().unwrap_or_else(|| "/dev/stdout".into());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("http_sink listening on {addr}");
    loop {
        let (mut sock, _) = listener.accept().await?;
        let capture = capture.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 1 << 16];
            let mut got = Vec::new();
            // Read headers, then the declared body.
            loop {
                let n = match sock.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                got.extend_from_slice(&buf[..n]);
                if let Some(end) = got.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&got[..end]).to_ascii_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                        .unwrap_or(0usize);
                    if got.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&capture) {
                let _ = f.write_all(&got);
                let _ = f.write_all(b"\n");
            }
            let _ = sock
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK")
                .await;
        });
    }
}
