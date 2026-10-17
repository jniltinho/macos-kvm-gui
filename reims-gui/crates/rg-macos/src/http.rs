//! Apple's endpoints over rg-core's shared HTTP client, with the Go app's timeouts and redirect
//! policy.

use std::io::Read;
use std::sync::atomic::AtomicBool;

use rg_core::http::{Client, ClientOpts};
use ureq::http::Response;

use crate::{Error, Result};

/// The HTTP clients this crate uses. Build one with [`Http::default`] and share it.
#[derive(Clone, Debug)]
pub struct Http {
    /// Apple's catalog, distributions and integrity data: HTTPS only (redirects too), 60 s.
    pub(crate) api: Client,
    /// The installer package: HTTPS only, no overall timeout (many GB), bounded connect/headers.
    pub(crate) download: Client,
    /// The recovery API (plain HTTP, as Apple serves it), 60 s.
    pub(crate) recovery_api: Client,
    /// The recovery image: no overall timeout, bounded connect/headers.
    pub(crate) recovery_download: Client,
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

fn client(download: bool, recovery: bool, roots: &[u8]) -> Client {
    Client::new(&ClientOpts {
        plain_http: recovery,
        download,
        user_agent: recovery.then(|| RECOVERY_USER_AGENT.to_owned()),
        roots: if roots.is_empty() {
            Vec::new()
        } else {
            vec![roots.to_vec()]
        },
    })
}

impl Default for Http {
    fn default() -> Self {
        Self {
            api: client(false, false, &[]),
            download: client(true, false, &[]),
            recovery_api: client(false, true, &[]),
            recovery_download: client(true, true, &[]),
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
        Self {
            api: client(false, false, root_der),
            download: client(true, false, root_der),
            recovery_base: recovery_base.to_owned(),
            ..Self::default()
        }
    }

    /// GET over HTTPS (redirects stay on HTTPS, 60 s), reading at most `limit` bytes; any
    /// status but 200 is an error (`httpsGet`).
    pub fn https_get(&self, url: &str, limit: u64) -> Result<Vec<u8>> {
        Ok(self.api.get(url, limit)?)
    }

    /// [`Http::https_get`] that fails, instead of truncating, past `max` bytes (`httpsGetMax`).
    pub fn https_get_max(&self, url: &str, max: u64) -> Result<Vec<u8>> {
        Ok(self.api.get_max(url, max, &AtomicBool::new(false))?)
    }
}

/// "403 Forbidden", like Go's `resp.Status`.
pub(crate) fn status_text<B>(resp: &Response<B>) -> String {
    rg_core::http::status_text(resp.status())
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
    Ok(rg_core::http::copy(r, w, cancel, each)?)
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
