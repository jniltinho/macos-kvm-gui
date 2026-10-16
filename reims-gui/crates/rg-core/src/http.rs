//! The one HTTP client (ureq 3 + rustls) behind every download: rg-assets' kexts, rg-macos'
//! recovery and installers, rg-reims' packages. Go's `httpsGet`/`httpsGetMax` and transports.
//!
//! It lives here, at layer 0, because those three crates are siblings or above: one copy, no
//! cycle. Each crate maps [`HttpError`] into its own error type.

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ureq::Agent;
use ureq::config::Config;

/// A failed request or body read.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    /// The cancel flag was set.
    #[error("cancelled")]
    Cancelled,
    /// Anything else, worded for the user (refused URL, network, status, size limit).
    #[error("{0}")]
    Failed(String),
}

fn failed(s: impl Into<String>) -> HttpError {
    HttpError::Failed(s.into())
}

/// How a [`Client`] is built.
#[derive(Clone, Debug, Default)]
pub struct ClientOpts {
    /// Allow plain `http://` (Apple's recovery API, tests' loopback servers). Off: HTTPS only,
    /// redirects too.
    pub plain_http: bool,
    /// Downloads of any size: 30 s to connect, 60 s to the response head, no overall timeout.
    /// Off: 60 s for the whole request (APIs, small files).
    pub download: bool,
    pub user_agent: Option<String>,
    /// DER certificates trusted instead of the web's roots (tests' local servers).
    pub roots: Vec<Vec<u8>>,
}

/// A ureq agent with the app's policy: at most 5 redirects, statuses returned (not errors),
/// proxies from the environment. Cheap to clone.
#[derive(Clone, Debug)]
pub struct Client {
    agent: Agent,
    https_only: bool,
}

impl Default for Client {
    /// HTTPS only, 60 s.
    fn default() -> Self {
        Self::new(&ClientOpts::default())
    }
}

impl Client {
    pub fn new(o: &ClientOpts) -> Self {
        let mut b = Config::builder()
            .http_status_as_error(false)
            .max_redirects(5)
            .https_only(!o.plain_http);
        b = if o.download {
            b.timeout_connect(Some(Duration::from_secs(30)))
                .timeout_recv_response(Some(Duration::from_secs(60)))
        } else {
            b.timeout_global(Some(Duration::from_secs(60)))
        };
        if let Some(ua) = &o.user_agent {
            b = b.user_agent(ua);
        }
        if !o.roots.is_empty() {
            use ureq::tls::{Certificate, RootCerts, TlsConfig};
            let certs = o
                .roots
                .iter()
                .map(|d| Certificate::from_der(d).to_owned())
                .collect();
            b = b.tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::Specific(Arc::new(certs)))
                    .build(),
            );
        }
        Self {
            agent: b.build().new_agent(),
            https_only: !o.plain_http,
        }
    }

    /// The agent, for requests that need more than a GET (headers, POST, ranges).
    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    /// Opens `url` with GET; any status but 200 is an error ("404 Not Found").
    pub fn open(&self, url: &str) -> Result<impl Read + use<>, HttpError> {
        if self.https_only && !url.starts_with("https://") {
            return Err(failed(format!("refusing a non-HTTPS URL: {url}")));
        }
        let resp = self
            .agent
            .get(url)
            .call()
            .map_err(|e| failed(format!("{url}: {e}")))?;
        if resp.status() != 200 {
            return Err(failed(format!("{url}: {}", status_text(resp.status()))));
        }
        Ok(resp.into_body().into_reader())
    }

    /// Go's `httpsGet`: the body, truncated to `limit` bytes.
    pub fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, HttpError> {
        let mut out = Vec::new();
        self.open(url)?
            .take(limit)
            .read_to_end(&mut out)
            .map_err(|e| failed(format!("{url}: {e}")))?;
        Ok(out)
    }

    /// Go's `httpsGetMax`: the body; more than `max` bytes is an error, never a truncation.
    /// `cancel` is checked before the request and between reads.
    pub fn get_max(&self, url: &str, max: u64, cancel: &AtomicBool) -> Result<Vec<u8>, HttpError> {
        let mut out = Vec::new();
        self.get_to(url, max, &mut out, cancel)?;
        Ok(out)
    }

    /// Streams the body into `w`; more than `max` bytes is an error. Returns the byte count.
    pub fn get_to(
        &self,
        url: &str,
        max: u64,
        w: &mut dyn Write,
        cancel: &AtomicBool,
    ) -> Result<u64, HttpError> {
        check(cancel)?;
        let mut body = self.open(url)?.take(max + 1);
        let n = copy(&mut body, w, cancel, &mut |_| {}).map_err(|e| prefix(url, e))?;
        if n > max {
            return Err(failed(format!("{url} is larger than {} MiB", max >> 20)));
        }
        Ok(n)
    }
}

fn prefix(url: &str, e: HttpError) -> HttpError {
    match e {
        HttpError::Failed(s) => failed(format!("{url}: {s}")),
        c => c,
    }
}

fn check(cancel: &AtomicBool) -> Result<(), HttpError> {
    if cancel.load(Ordering::Relaxed) {
        Err(HttpError::Cancelled)
    } else {
        Ok(())
    }
}

/// "403 Forbidden", like Go's `resp.Status`.
pub fn status_text(s: ureq::http::StatusCode) -> String {
    format!("{} {}", s.as_u16(), s.canonical_reason().unwrap_or(""))
}

/// Copies `r` into `w` in 1 MiB reads, checking `cancel` between reads and calling `each(n)`
/// with the bytes copied so far.
pub fn copy(
    r: &mut dyn Read,
    w: &mut dyn Write,
    cancel: &AtomicBool,
    each: &mut dyn FnMut(u64),
) -> Result<u64, HttpError> {
    let mut buf = vec![0; 1 << 20];
    let mut n = 0u64;
    loop {
        check(cancel)?;
        let k = match r.read(&mut buf) {
            Ok(0) => return Ok(n),
            Ok(k) => k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(failed(e.to_string())),
        };
        w.write_all(&buf[..k]).map_err(|e| failed(e.to_string()))?;
        n += k as u64;
        each(n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_stops_when_cancelled_and_counts_bytes() {
        let no = AtomicBool::new(false);
        let mut out = Vec::new();
        let mut seen = 0;
        let n = copy(&mut &b"hello"[..], &mut out, &no, &mut |n| seen = n);
        assert!(matches!(n, Ok(5)) && seen == 5 && out == b"hello");
        let yes = AtomicBool::new(true);
        assert!(matches!(
            copy(&mut &b"x"[..], &mut Vec::new(), &yes, &mut |_| {}),
            Err(HttpError::Cancelled)
        ));
    }

    #[test]
    fn https_only_client_refuses_plain_urls() {
        let e = Client::default()
            .get("http://example.invalid/", 10)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(e.contains("non-HTTPS"), "{e}");
    }
}
