//! HTTPS downloads with a size limit that fails instead of truncating (Go `httpsGetMax`),
//! through rg-core's shared client.

use std::sync::atomic::AtomicBool;

use rg_core::http::{Client, ClientOpts, HttpError};

use crate::{Error, Result};

/// Fetches `url` (https:// only, redirects too) into memory; more than `max` bytes is an
/// error. `cancel` is checked before the request and between reads.
pub(crate) fn https_get_max(url: &str, max: u64, cancel: &AtomicBool) -> Result<Vec<u8>> {
    // This crate's tests serve over plain HTTP on loopback.
    let allowed = url.starts_with("https://") || (cfg!(test) && url.starts_with("http://127."));
    if !allowed {
        return Err(Error::Http(format!("refusing a non-HTTPS URL: {url}")));
    }
    let client = Client::new(&ClientOpts {
        plain_http: cfg!(test),
        ..ClientOpts::default()
    });
    client.get_max(url, max, cancel).map_err(|e| match e {
        HttpError::Cancelled => Error::Cancelled,
        HttpError::Failed(s) => Error::Http(s),
    })
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
