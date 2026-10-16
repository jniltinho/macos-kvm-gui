//! HTTPS downloads with a size limit that fails instead of truncating (Go `httpsGetMax`).

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::{Error, Result};

/// Fetches `url` (https:// only, redirects too) into memory; more than `max` bytes is an
/// error. `cancel` is checked before the request and between reads.
pub(crate) fn https_get_max(url: &str, max: u64, cancel: &AtomicBool) -> Result<Vec<u8>> {
    // This crate's tests serve over plain HTTP on loopback.
    let allowed = url.starts_with("https://") || (cfg!(test) && url.starts_with("http://127."));
    if !allowed {
        return Err(Error::Http(format!("refusing a non-HTTPS URL: {url}")));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(!cfg!(test))
        .max_redirects(5)
        .timeout_global(Some(Duration::from_secs(60)))
        .build()
        .into();
    let mut resp = agent
        .get(url)
        .call()
        .map_err(|e| Error::Http(format!("{url}: {e}")))?;
    if resp.status() != 200 {
        return Err(Error::Http(format!("{url}: {}", resp.status())));
    }
    let mut body = resp.body_mut().as_reader();
    let mut data = Vec::new();
    let mut buf = vec![0; 64 << 10];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let n = body.read(&mut buf)?;
        if n == 0 {
            return Ok(data);
        }
        if (data.len() + n) as u64 > max {
            return Err(Error::Http(format!(
                "{url} is larger than {} MiB",
                max >> 20
            )));
        }
        data.extend_from_slice(&buf[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::testutil::serve;

    #[test]
    fn get_max_fails_past_the_limit_and_accepts_at_it() -> Result<()> {
        let (base, _) = serve(
            [
                ("/big".to_string(), vec![0; 11]),
                ("/ok".to_string(), vec![0; 10]),
            ]
            .into(),
        );
        let no = AtomicBool::new(false);
        let err = https_get_max(&format!("{base}/big"), 10, &no)
            .err()
            .map(|e| e.to_string());
        assert!(err.unwrap_or_default().contains("larger"));
        assert_eq!(https_get_max(&format!("{base}/ok"), 10, &no)?.len(), 10);
        Ok(())
    }

    #[test]
    fn get_max_refuses_http_status_and_plain_urls() {
        let (base, _) = serve([].into());
        let no = AtomicBool::new(false);
        assert!(https_get_max(&format!("{base}/missing"), 10, &no).is_err());
        assert!(https_get_max("http://example.com/x", 10, &no).is_err());
        assert!(matches!(
            https_get_max(&format!("{base}/x"), 10, &AtomicBool::new(true)),
            Err(Error::Cancelled)
        ));
    }
}
