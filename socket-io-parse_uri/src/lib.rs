/// Parses URIs for the socket.io client
///
/// Adapted from https://github.com/galkn/parseuri
use fancy_regex::Regex;
use lazy_static::lazy_static;
use thiserror::Error;

lazy_static! {
    static ref RE: Regex = {
        let pattern = r#"^(?:(?![^:@]+:[^:@\/]*@)(http|https|ws|wss):\/\/)?((?:(([^:@]*)(?::([^:@]*))?)?@)?((?:[a-f0-9]{0,4}:){2,7}[a-f0-9]{0,4}|[^:\/?#]*)(?::(\d*))?)(((\/(?:[^?#](?![^?#\/]*\.[^?#\/.]+(?:[?#]|$)))*\/?)?([^?#\/]*))(?:\?([^#]*))?(?:#(.*))?)"#;
        Regex::new(pattern).unwrap()
    };
}

pub struct Uri<'a> {
    host: &'a str,
    port: u16,
    secure: bool,
    path: &'a str,
    query: Option<&'a str>,
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("Invalid URI: {0}")]
    InvalidUri(String),
    #[error("Invalid port: {0}")]
    InvalidPort(String),
    #[error("No host: {0}")]
    NoHost(String),
    #[error("Unexpected error matching URI {0}: {1}")]
    MatchError(String, fancy_regex::Error),
}

pub fn parse_uri<'a, T: AsRef<str> + ?Sized>(uri: &'a T) -> Result<Uri<'a>, ParseError> {
    let uri = uri.as_ref();
    let invalid_uri = || ParseError::InvalidUri(uri.to_string());
    let captures = RE
        .captures(uri)
        .map_err(|e| ParseError::MatchError(uri.to_string(), e))?
        .ok_or_else(invalid_uri)?;
    let protocol = captures.get(1).ok_or_else(invalid_uri)?;
    let (secure, port) = match protocol.as_str() {
        "ws" | "http" => (false, 80),
        "wss" | "https" => (true, 443),
        _ => unreachable!(),
    };
    let host = captures.get(6).ok_or_else(invalid_uri)?.as_str();
    let port = captures
        .get(7)
        .map(|x| -> Result<u16, ParseError> {
            let s = x.as_str();
            s.parse()
                .map_err(|_| ParseError::InvalidPort(s.to_string()))
        })
        .unwrap_or(Ok(port))?;
    let path = captures.get(10).ok_or_else(invalid_uri)?.as_str();
    let query = captures.get(12).map(|x| x.as_str());
    Ok(Uri {
        host,
        port,
        secure,
        path,
        query,
    })
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_re1() {
        assert!(RE
            .is_match("wss://localhost:3000/socket.io/file?query=a")
            .unwrap());
    }

    #[test]
    fn test_re2() {
        let captures = RE
            .captures("wss://localhost:3000/socket.io/file?query=a")
            .unwrap()
            .unwrap();
        let get = |i| captures.get(i).map(|x| x.as_str());
        assert_eq!(captures.len(), 14);
        assert_eq!(get(1), Some("wss"));
        assert_eq!(get(2), Some("localhost:3000"));
        assert_eq!(get(3), None);
        assert_eq!(get(4), None);
        assert_eq!(get(5), None);
        assert_eq!(get(6), Some("localhost"));
        assert_eq!(get(7), Some("3000"));
        assert_eq!(get(8), Some("/socket.io/file?query=a"));
        assert_eq!(get(9), Some("/socket.io/file"));
        assert_eq!(get(10), Some("/socket.io/file"));
        assert_eq!(get(11), Some(""));
        assert_eq!(get(12), Some("query=a"));
        assert_eq!(get(13), None);
    }

    #[test]
    fn test_re_noprotocol() {
        assert!(RE
            .is_match("localhost:3000/socket.io/file?query=a")
            .unwrap());
    }

    #[test]
    fn test_parse_uri() {
        let p = parse_uri("wss://example.com/").unwrap();
        assert_eq!(p.host, "example.com");
        assert_eq!(p.secure, true);
        assert_eq!(p.path, "/");
        assert_eq!(p.query, None);
    }
}
