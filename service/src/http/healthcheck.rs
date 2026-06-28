use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::errors::AppError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpHealthcheckOptions {
    pub url: String,
    pub timeout: Duration,
}

impl Default for HttpHealthcheckOptions {
    fn default() -> Self {
        Self {
            url: "http://127.0.0.1:8080/ready".to_owned(),
            timeout: Duration::from_secs(3),
        }
    }
}

#[derive(Debug)]
struct HttpTarget {
    host: String,
    port: u16,
    path: String,
}

pub fn run_http_healthcheck(options: &HttpHealthcheckOptions) -> Result<(), AppError> {
    let target = parse_http_url(&options.url)?;
    let mut addrs = (target.host.as_str(), target.port)
        .to_socket_addrs()
        .map_err(|error| {
            AppError::service_unavailable(format!("Healthcheck DNS failed: {error}"))
        })?;
    let addr = addrs.next().ok_or_else(|| {
        AppError::service_unavailable(format!(
            "Healthcheck host resolved no addresses: {}",
            target.host
        ))
    })?;
    let mut stream = TcpStream::connect_timeout(&addr, options.timeout).map_err(|error| {
        AppError::service_unavailable(format!(
            "Healthcheck connection failed for {}: {error}",
            options.url
        ))
    })?;
    stream
        .set_read_timeout(Some(options.timeout))
        .map_err(AppError::from)?;
    stream
        .set_write_timeout(Some(options.timeout))
        .map_err(AppError::from)?;

    let host = if target.port == 80 {
        target.host.clone()
    } else {
        format!("{}:{}", target.host, target.port)
    };
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: poker-hands-storage-healthcheck\r\nConnection: close\r\n\r\n",
        target.path, host
    );
    stream
        .write_all(request.as_bytes())
        .map_err(AppError::from)?;
    let status = read_status_code(&mut stream)?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(AppError::service_unavailable(format!(
            "Healthcheck failed for {}: HTTP {}",
            options.url, status
        )))
    }
}

fn parse_http_url(url: &str) -> Result<HttpTarget, AppError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| AppError::invalid_argument("healthcheck URL must use http://"))?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/".to_owned()),
    };
    if authority.is_empty() {
        return Err(AppError::invalid_argument(
            "healthcheck URL must include a host",
        ));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() => {
            let port = port
                .parse::<u16>()
                .map_err(|_| AppError::invalid_argument("healthcheck URL port must be numeric"))?;
            (host.to_owned(), port)
        }
        _ => (authority.to_owned(), 80),
    };
    Ok(HttpTarget { host, port, path })
}

fn read_status_code(stream: &mut TcpStream) -> Result<u16, AppError> {
    let mut response = Vec::with_capacity(256);
    let mut buffer = [0_u8; 256];
    while response.len() < 4096 {
        let read = stream.read(&mut buffer).map_err(AppError::from)?;
        if read == 0 {
            break;
        }
        response.extend_from_slice(&buffer[..read]);
        if response.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    let response = String::from_utf8_lossy(&response);
    let status_line = response
        .lines()
        .next()
        .ok_or_else(|| AppError::service_unavailable("Healthcheck returned an empty response"))?;
    status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| {
            AppError::service_unavailable(format!(
                "Healthcheck returned an invalid status line: {status_line}"
            ))
        })?
        .parse::<u16>()
        .map_err(|_| {
            AppError::service_unavailable(format!(
                "Healthcheck returned a non-numeric status line: {status_line}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    #[test]
    fn healthcheck_accepts_success_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 256];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });

        run_http_healthcheck(&HttpHealthcheckOptions {
            url: format!("http://127.0.0.1:{}/ready", addr.port()),
            timeout: Duration::from_secs(1),
        })
        .unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn healthcheck_rejects_non_success_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 256];
            let _ = stream.read(&mut request).unwrap();
            stream
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
        });

        let error = run_http_healthcheck(&HttpHealthcheckOptions {
            url: format!("http://127.0.0.1:{}/ready", addr.port()),
            timeout: Duration::from_secs(1),
        })
        .unwrap_err();
        assert!(error.message().contains("HTTP 503"));
        handle.join().unwrap();
    }

    #[test]
    fn healthcheck_rejects_non_http_url() {
        let error = parse_http_url("https://127.0.0.1:8080/ready").unwrap_err();
        assert!(error.message().contains("http://"));
    }
}
