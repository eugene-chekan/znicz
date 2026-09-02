#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioAddictNetwork {
    RadioTunes,
    Di,
    RockRadio,
    JazzRadio,
    ClassicalRadio,
    ZenRadio,
}

impl AudioAddictNetwork {
    pub fn slug(self) -> &'static str {
        match self {
            Self::RadioTunes => "radiotunes",
            Self::Di => "di",
            Self::RockRadio => "rockradio",
            Self::JazzRadio => "jazzradio",
            Self::ClassicalRadio => "classicalradio",
            Self::ZenRadio => "zenradio",
        }
    }

    fn from_host(host: &str) -> Option<Self> {
        const PAIRS: &[(&str, AudioAddictNetwork)] = &[
            ("radiotunes.com", AudioAddictNetwork::RadioTunes),
            ("di.fm", AudioAddictNetwork::Di),
            ("rockradio.com", AudioAddictNetwork::RockRadio),
            ("jazzradio.com", AudioAddictNetwork::JazzRadio),
            ("classicalradio.com", AudioAddictNetwork::ClassicalRadio),
            ("zenradio.com", AudioAddictNetwork::ZenRadio),
        ];
        let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
        for (domain, net) in PAIRS {
            if host == *domain || host.ends_with(&format!(".{domain}")) {
                return Some(*net);
            }
        }
        None
    }
}

const QUALITY_SUFFIXES: &[&str] = &["_aacplus", "_aac", "_premium", "_hi", "_med", "_low"];

pub fn parse_audioaddict_channel(stream_url: &str) -> Option<(AudioAddictNetwork, String)> {
    let url = stream_url.trim();
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))?;
    let authority_path = rest.split_once('?').map(|(a, _)| a).unwrap_or(rest);
    let (authority, path) = match authority_path.split_once('/') {
        Some((a, p)) => (a, p),
        None => (authority_path, ""),
    };
    let host = authority
        .rsplit_once('@')
        .map(|(_, h)| h)
        .unwrap_or(authority);
    let host = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
    if host.starts_with('[') {
        return None;
    }
    let network = AudioAddictNetwork::from_host(host)?;
    let segment = path.split('/').next().unwrap_or("");
    if segment.is_empty() {
        return None;
    }
    let mut key = segment.to_string();
    for suffix in QUALITY_SUFFIXES {
        if let Some(stripped) = key.strip_suffix(suffix) {
            if !stripped.is_empty() {
                key = stripped.to_string();
                break;
            }
        }
    }
    Some((network, key))
}

#[cfg(test)]
mod tests {
    use super::{parse_audioaddict_channel, AudioAddictNetwork};

    #[test]
    fn parse_radiotunes_hi_strips_suffix_and_ignores_query() {
        let (net, key) =
            parse_audioaddict_channel("http://prem2.radiotunes.com:80/datempolounge_hi?listenkey")
                .expect("radiotunes");
        assert_eq!(net, AudioAddictNetwork::RadioTunes);
        assert_eq!(key, "datempolounge");
    }

    #[test]
    fn parse_rockradio_path_without_suffix() {
        let (net, key) =
            parse_audioaddict_channel("http://prem2.rockradio.com:80/metal").expect("rockradio");
        assert_eq!(net, AudioAddictNetwork::RockRadio);
        assert_eq!(key, "metal");
    }

    #[test]
    fn parse_di_fm_hi() {
        let (net, key) =
            parse_audioaddict_channel("http://prem2.di.fm:80/lofiloungenchill_hi?listenkey")
                .expect("di");
        assert_eq!(net, AudioAddictNetwork::Di);
        assert_eq!(key, "lofiloungenchill");
    }

    #[test]
    fn parse_rejects_unknown_host_and_non_http() {
        assert!(parse_audioaddict_channel("https://example.com/x").is_none());
        assert!(parse_audioaddict_channel("file:///tmp/x").is_none());
        assert!(parse_audioaddict_channel("").is_none());
    }

    #[test]
    fn parse_aacplus_wins_over_aac() {
        let (_, key) =
            parse_audioaddict_channel("https://listen.jazzradio.com/cool_aacplus").expect("jazz");
        assert_eq!(key, "cool");
        let (_, key) = parse_audioaddict_channel("https://listen.classicalradio.com/baroque_aac")
            .expect("classical");
        assert_eq!(key, "baroque");
    }

    #[test]
    fn parse_unknown_suffix_stays() {
        let (_, key) =
            parse_audioaddict_channel("https://prem1.zenradio.com/foo_bar").expect("zen");
        assert_eq!(key, "foo_bar");
    }

    #[test]
    fn parse_host_is_domain_or_subdomain_not_suffix_spam() {
        assert!(parse_audioaddict_channel("http://notactuallyradiotunes.com/x").is_none());
    }
}
