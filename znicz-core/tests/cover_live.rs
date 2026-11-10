//! Live check of the stream cover chain against Deezer and MusicBrainz.
//!
//! Needs network access, so it is ignored by default:
//! `cargo test -p znicz-core --test cover_live -- --ignored --nocapture`

use znicz_core::{default_sources, Resolution, StreamCovers, StreamQuery};

#[test]
#[ignore = "hits Deezer and MusicBrainz"]
fn live_covers_for_known_stations() {
    let covers = StreamCovers::new(default_sources());
    let queries = [
        StreamQuery {
            stream_url: "https://channels.fluxfm.de/jazz-schwarzenstein/stream.mp3".into(),
            station_name: "FluxFM - Jazzradio Schwarzenstein".into(),
            icy_title: Some("Ben Webster - Willow Weep For Me".into()),
            icy_url: None,
            station_art: None,
            search: true,
        },
        StreamQuery {
            stream_url: "http://stream.heavy-music.net:8176/stream".into(),
            station_name: "HEAVY-MUSIC.NET".into(),
            icy_title: Some("Serotonal - Memory Leaves".into()),
            icy_url: None,
            station_art: None,
            search: true,
        },
    ];

    for query in &queries {
        match covers.resolve(query) {
            Resolution::Found(art) => {
                eprintln!(
                    "{}: {} ({} bytes)",
                    query.station_name,
                    art.mime,
                    art.bytes.len()
                );
                assert!(
                    art.mime.starts_with("image/"),
                    "{}: not an image: {}",
                    query.station_name,
                    art.mime
                );
                assert!(!art.bytes.is_empty(), "{}: empty cover", query.station_name);
            }
            other => panic!("{}: expected Found, got {other:?}", query.station_name),
        }
    }
}
