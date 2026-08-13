//! A tiny, dependency-free Prometheus metrics endpoint.
//!
//! `toptop --serve-metrics` turns the monitor into a long-running exporter: a
//! background thread refreshes a [`Collector`] and re-renders the Prometheus
//! text into a shared buffer; the HTTP loop serves it on `/metrics`. This is the
//! "observability source" mode — point Prometheus/VictoriaMetrics at it and
//! chart tokens/sec, VRAM pressure, throttle events and alerts over time.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::config::Config;
use crate::export;
use crate::metrics::Collector;

const INDEX: &str = "<!doctype html><title>toptop</title>\
<body style=\"font-family:monospace;background:#0d0a1a;color:#dcebff\">\
<h2>toptop metrics exporter</h2>\
<p><a style=\"color:#2de2e6\" href=\"/metrics\">/metrics</a> — Prometheus exposition format</p>\
<p><a style=\"color:#2de2e6\" href=\"/healthz\">/healthz</a></p></body>";

/// Serve Prometheus metrics on `addr` until the process is killed.
pub fn run(addr: &str, cfg: &Config) -> std::io::Result<()> {
    let shared = Arc::new(Mutex::new(String::from("# toptop: warming up\n")));
    let interval = Duration::from_millis(cfg.tick_ms.max(1000));
    let alert_cfg = cfg.alerts.clone();

    // Background sampler: owns the Collector, republishes rendered metrics.
    {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("toptop-exporter".into())
            .spawn(move || {
                let mut c = Collector::new(256);
                loop {
                    c.refresh();
                    let text = export::to_prometheus(&c, &alert_cfg);
                    if let Ok(mut s) = shared.lock() {
                        *s = text;
                    }
                    std::thread::sleep(interval);
                }
            })?;
    }

    let listener = TcpListener::bind(addr)?;
    eprintln!("toptop: serving Prometheus metrics at http://{addr}/metrics (Ctrl-C to stop)");
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut buf = [0u8; 2048];
        let n = stream.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..n]);
        let path = req.split_whitespace().nth(1).unwrap_or("/");
        let (status, ctype, body) = match path {
            "/metrics" => (
                "200 OK",
                "text/plain; version=0.0.4; charset=utf-8",
                shared.lock().map(|s| s.clone()).unwrap_or_default(),
            ),
            "/healthz" => ("200 OK", "text/plain", "ok\n".to_string()),
            "/" => ("200 OK", "text/html; charset=utf-8", INDEX.to_string()),
            _ => ("404 Not Found", "text/plain", "not found\n".to_string()),
        };
        let resp = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(resp.as_bytes());
    }
    Ok(())
}
