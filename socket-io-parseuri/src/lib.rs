use fancy_regex::Regex;
/// Parses URIs for the socket.io client
///
/// Adapted from https://github.com/galkn/parseuri
use lazy_static::lazy_static;

lazy_static! {
    static ref RE: Regex = {
        let pattern = r#"^(?:(?![^:@]+:[^:@\/]*@)(http|https|ws|wss):\/\/)?((?:(([^:@]*)(?::([^:@]*))?)?@)?((?:[a-f0-9]{0,4}:){2,7}[a-f0-9]{0,4}|[^:\/?#]*)(?::(\d*))?)(((\/(?:[^?#](?![^?#\/]*\.[^?#\/.]+(?:[?#]|$)))*\/?)?([^?#\/]*))(?:\?([^#]*))?(?:#(.*))?)"#;
        Regex::new(pattern).unwrap()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_re1() {
        assert!(RE
            .is_match("wss://localhost:3000/socket.io/&query=a")
            .unwrap());
    }
}
