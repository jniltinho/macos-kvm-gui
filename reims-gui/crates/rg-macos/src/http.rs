//! HTTP clients (ureq + rustls) with the Go app's timeouts and redirect policy.

use std::io::Read;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use ureq::Agent;
use ureq::config::Config;
use ureq::http::Response;

use crate::{Error, Result, other};

/// The HTTP clients this crate uses. Build one with [`Http::default`] and share it.
#[derive(Clone, Debug)]
pub struct Http {
    /// Apple's catalog, distributions and integrity data: HTTPS only (redirects too), 60 s.
    pub(crate) api: Agent,
    /// The installer package: HTTPS only, no overall timeout (many GB), bounded connect/headers.
    pub(crate) download: Agent,
    /// The recovery API (plain HTTP, as Apple serves it), 60 s.
    pub(crate) recovery_api: Agent,
    /// The recovery image: no overall timeout, bounded connect/headers.
    pub(crate) recovery_download: Agent,
    /// `http://osrecovery.apple.com`; tests point it at a local server (the Host header stays).
    pub(crate) recovery_base: String,
    /// Apple's catalogs, newest first.
    pub(crate) catalogs: Vec<String>,
}

const RECOVERY_USER_AGENT: &str = "InternetRecovery/1.0";

/// Mist's public-release catalogs, newest first; the Tahoe one lists every InstallAssistant
/// from Big Sur on.
const INSTALLER_CATALOGS: [&str; 2] = [
    "https://swscan.apple.com/content/catalogs/others/index-26-15-14-13-12-10.16-10.15-10.14-10.13-10.12-10.11-10.10-10.9-mountainlion-lion-snowleopard-leopard.merged-1.sucatalog.gz",
    "https://swscan.apple.com/content/catalogs/others/index-26-15-14-13-12-10.16-10.15-10.14-10.13-10.12-10.11-10.10-10.9-mountainlion-lion-snowleopard-leopard.merged-1.sucatalog",
];

fn base() -> ureq::config::ConfigBuilder<ureq::typestate::AgentScope> {
    Config::builder()
        .http_status_as_error(false)
        .max_redirects(5)
}

fn api(b: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>) -> Agent {
    b.timeout_global(Some(Duration::from_secs(60)))
        .build()
        .new_agent()
}

fn download(b: ureq::config::ConfigBuilder<ureq::typestate::AgentScope>) -> Agent {
    b.timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .build()
        .new_agent()
}

impl Default for Http {
    fn default() -> Self {
        Self {
            api: api(base().https_only(true)),
            download: download(base().https_only(true)),
            recovery_api: api(base().user_agent(RECOVERY_USER_AGENT)),
            recovery_download: download(base().user_agent(RECOVERY_USER_AGENT)),
            recovery_base: "http://osrecovery.apple.com".into(),
            catalogs: INSTALLER_CATALOGS.iter().map(|s| (*s).to_owned()).collect(),
        }
    }
}

impl Http {
    /// Clients that also trust `root_der` (a test server's self-signed certificate) and send
    /// recovery requests to `recovery_base`.
    #[cfg(test)]
    pub(crate) fn for_tests(root_der: &[u8], recovery_base: &str) -> Self {
        use ureq::tls::{Certificate, RootCerts, TlsConfig};
        let b = || {
            let b = base().https_only(true);
            if root_der.is_empty() {
                return b;
            }
            let certs = RootCerts::new_with_certs(&[Certificate::from_der(root_der).to_owned()]);
            b.tls_config(TlsConfig::builder().root_certs(certs).build())
        };
        Self {
            api: api(b()),
            download: download(b()),
            recovery_base: recovery_base.to_owned(),
            ..Self::default()
        }
    }

    /// GET over HTTPS (redirects stay on HTTPS, 60 s), reading at most `limit` bytes; any
    /// status but 200 is an error (`httpsGet`, also used by the kext downloads in rg-assets).
    pub fn https_get(&self, url: &str, limit: u64) -> Result<Vec<u8>> {
        if !url.starts_with("https://") {
            return other(format!("refusing a non-HTTPS URL: {url}"));
        }
        let resp = self.api.get(url).call().map_err(|e| http_err(url, e))?;
        if resp.status() != 200 {
            return other(format!("{url}: {}", status_text(&resp)));
        }
        let mut out = Vec::new();
        resp.into_body()
            .into_reader()
            .take(limit)
            .read_to_end(&mut out)?;
        Ok(out)
    }

    /// [`Http::https_get`] that fails, instead of truncating, past `max` bytes (`httpsGetMax`).
    pub fn https_get_max(&self, url: &str, max: u64) -> Result<Vec<u8>> {
        let data = self.https_get(url, max + 1)?;
        if data.len() as u64 > max {
            return other(format!("{url} is larger than {} MiB", max >> 20));
        }
        Ok(data)
    }
}

/// "403 Forbidden", like Go's `resp.Status`.
pub(crate) fn status_text<B>(resp: &Response<B>) -> String {
    let s = resp.status();
    format!("{} {}", s.as_u16(), s.canonical_reason().unwrap_or(""))
}

pub(crate) fn http_err(what: &str, e: ureq::Error) -> Error {
    Error::Other(format!("{what}: {e}"))
}

/// Copies `r` into `w` in 1 MiB reads, checking `cancel` between reads and calling `each(n)`
/// with the bytes copied so far.
pub(crate) fn copy_body(
    r: &mut dyn Read,
    w: &mut dyn std::io::Write,
    cancel: &AtomicBool,
    each: &mut dyn FnMut(u64),
) -> Result<u64> {
    let mut buf = vec![0; 1 << 20];
    let mut n = 0u64;
    loop {
        crate::check_cancel(cancel)?;
        let r = match r.read(&mut buf) {
            Ok(0) => return Ok(n),
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        w.write_all(&buf[..r])?;
        n += r as u64;
        each(n);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testserver::{Reply, Server};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn https_get_max_refuses_larger_bodies_and_plain_http() -> TestResult {
        let srv = Server::https(|_| Reply::ok(vec![b'x'; 2 << 20]))?;
        let http = Http::for_tests(&srv.cert_der, "");
        assert_eq!(http.https_get_max(&srv.url("/"), 4 << 20)?.len(), 2 << 20);
        assert!(
            http.https_get_max(&srv.url("/"), 1 << 20)
                .is_err_and(|e| e.to_string().contains("larger"))
        );
        assert!(
            http.https_get("http://example.invalid/", 10)
                .is_err_and(|e| e.to_string().contains("non-HTTPS"))
        );
        Ok(())
    }

    #[test]
    fn redirects_stay_on_https() -> TestResult {
        let plain = Server::http(|_| Reply::ok(b"plain".to_vec()))?;
        let to = plain.url("/x");
        let srv = Server::https(move |_| {
            Reply::new(302, vec![("Location".into(), to.clone())], Vec::new())
        })?;
        let http = Http::for_tests(&srv.cert_der, "");
        let e = http
            .https_get(&srv.url("/"), 100)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            e.contains("https only"),
            "a redirect to HTTP must be refused: {e}"
        );
        Ok(())
    }
}
