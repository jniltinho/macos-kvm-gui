//! A tiny HTTP/1.1 test server (plain or HTTPS with a self-signed rcgen certificate), standing in
//! for Go's httptest. One request per connection.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;

type BoxError = Box<dyn std::error::Error>;

/// A request as the handler sees it; header names are lower case.
#[derive(Debug, Default)]
pub(crate) struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> &str {
        self.headers
            .iter()
            .find(|(k, _)| k == name)
            .map_or("", |(_, v)| v.as_str())
    }
}

pub(crate) struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn new(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        Self {
            status,
            headers,
            body,
        }
    }

    pub fn ok(body: Vec<u8>) -> Self {
        Self::new(200, Vec::new(), body)
    }

    /// Go's http.ServeContent for `bytes=N-` ranges: 206, 416, or 200 with everything.
    pub fn content(req: &Request, data: &[u8]) -> Self {
        let Some(from) = req
            .header("range")
            .strip_prefix("bytes=")
            .and_then(|r| r.strip_suffix('-'))
        else {
            return Self::ok(data.to_vec());
        };
        match from.parse::<usize>() {
            Ok(n) if n < data.len() => Self::new(
                206,
                vec![(
                    "Content-Range".into(),
                    format!("bytes {n}-{}/{}", data.len() - 1, data.len()),
                )],
                data[n..].to_vec(),
            ),
            _ => Self::new(
                416,
                vec![("Content-Range".into(), format!("bytes */{}", data.len()))],
                Vec::new(),
            ),
        }
    }
}

type Handler = dyn Fn(&Request) -> Reply + Send + Sync;

pub(crate) struct Server {
    pub base: String,
    /// The certificate to trust (empty for plain HTTP).
    pub cert_der: Vec<u8>,
}

impl Server {
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub fn http(h: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Result<Self, BoxError> {
        Self::start(None, Arc::new(h), Vec::new())
    }

    pub fn https(h: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Result<Self, BoxError> {
        let ck = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()])?;
        let cert = ck.cert.der().clone();
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(ck.signing_key.serialize_der().into());
        let cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![cert.clone()], key)?;
        Self::start(Some(Arc::new(cfg)), Arc::new(h), cert.to_vec())
    }

    fn start(
        tls: Option<Arc<rustls::ServerConfig>>,
        h: Arc<Handler>,
        cert_der: Vec<u8>,
    ) -> Result<Self, BoxError> {
        let ln = TcpListener::bind("127.0.0.1:0")?;
        let scheme = if tls.is_some() { "https" } else { "http" };
        let base = format!("{scheme}://{}", ln.local_addr()?);
        std::thread::spawn(move || {
            for conn in ln.incoming().flatten() {
                let (h, tls) = (h.clone(), tls.clone());
                std::thread::spawn(move || {
                    // A client that gives up (or refuses the redirect) is not a test failure.
                    let _ = serve(conn, tls, &*h);
                });
            }
        });
        Ok(Self { base, cert_der })
    }
}

fn serve(
    conn: TcpStream,
    tls: Option<Arc<rustls::ServerConfig>>,
    h: &Handler,
) -> Result<(), BoxError> {
    match tls {
        None => exchange(conn, h),
        Some(cfg) => {
            let mut s = rustls::StreamOwned::new(rustls::ServerConnection::new(cfg)?, conn);
            exchange(&mut s, h)?;
            s.conn.send_close_notify();
            s.flush()?;
            Ok(())
        }
    }
}

fn exchange(mut s: impl Read + Write, h: &Handler) -> Result<(), BoxError> {
    let mut r = BufReader::new(&mut s);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let mut req = Request {
        method: parts.next().unwrap_or_default().to_owned(),
        path: parts.next().unwrap_or_default().to_owned(),
        ..Request::default()
    };
    loop {
        line.clear();
        r.read_line(&mut line)?;
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            req.headers
                .push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
        }
    }
    let len = req.header("content-length").parse().unwrap_or(0);
    req.body = vec![0; len];
    r.read_exact(&mut req.body)?;
    drop(r);

    let reply = h(&req);
    let mut head = format!("HTTP/1.1 {} X\r\nConnection: close\r\n", reply.status);
    if !reply
        .headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("content-length"))
    {
        head += &format!("Content-Length: {}\r\n", reply.body.len());
    }
    for (k, v) in &reply.headers {
        head += &format!("{k}: {v}\r\n");
    }
    head += "\r\n";
    s.write_all(head.as_bytes())?;
    s.write_all(&reply.body)?;
    s.flush()?;
    Ok(())
}
