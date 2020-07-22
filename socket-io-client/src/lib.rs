use http::uri::{InvalidUri, Uri};
use tokio_tungstenite as ws;

struct Client {}

#[derive(thiserror::Error, Debug)]
enum Error {
    #[error("Failed to parse URI {0}: {1}")]
    UriError(String, UriError),
}

#[derive(thiserror::Error, Debug)]
enum UriError {
    #[error(transparent)]
    Parse(#[from] InvalidUri),
    #[error("Invalid scheme: {0:?}")]
    InvalidScheme(Option<String>),
    #[error("No host")]
    NoHost,
}

impl Client {
    pub async fn connect(uri: impl AsRef<str>) -> Result<Client, Error> {
        let uri = uri.as_ref();
        let uri = parse_uri(uri).map_err(|e| Error::UriError(uri.to_string(), e))?;
        unimplemented!();
    }
}

fn parse_uri(uri: &str) -> Result<Uri, UriError> {
    use std::convert::TryFrom;

    let uri = Uri::try_from(uri)?;

    let (scheme, port) = match uri.scheme_str() {
        Some("http") | Some("ws") => ("ws", 80),
        Some("https") | Some("wss") => ("wss", 443),
        s => return Err(UriError::InvalidScheme(s.map(|s| s.to_string()))),
    };

    let host = uri.host().ok_or(UriError::NoHost)?;
    let port = uri.port_u16().unwrap_or(port);
    let path_and_query = uri.path_and_query().map(|x| x.as_str()).unwrap_or("/");
    Ok(Uri::try_from(format!("{}://{}:{}{}", scheme, host, port, path_and_query)).unwrap())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_uri() {
        let p = parse_uri("https://example.com/").unwrap();
        assert_eq!(p, "wss://example.com:443/");
    }
}
