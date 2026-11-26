//! The accounts service: where the user approves, and how a code or a
//! refresh token becomes an access token.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::Value;

use crate::error::{Result, SpotifyError};
use crate::http::{Body, Http, Method, Reply};
use crate::token::Token;

pub(crate) const SCOPES: &str = "user-read-private user-read-playback-state \
user-modify-playback-state user-read-currently-playing user-library-read \
playlist-read-private playlist-read-collaborative";

/// Characters left as they are in a query value (RFC 3986 unreserved).
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

fn encode(value: &str) -> String {
    utf8_percent_encode(value, QUERY_VALUE).to_string()
}

pub(crate) fn authorize_url(
    accounts_base: &str,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
) -> String {
    format!(
        "{accounts_base}/authorize?client_id={}&response_type=code&redirect_uri={}\
         &code_challenge_method=S256&code_challenge={}&state={}&scope={}",
        encode(client_id),
        encode(redirect_uri),
        encode(challenge),
        encode(state),
        encode(SCOPES),
    )
}

pub(crate) fn exchange_code(
    http: &Http,
    accounts_base: &str,
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    now_secs: u64,
) -> Result<Token> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ];
    let reply = post_token(http, accounts_base, &form)?;
    if reply.status != 200 {
        let (error, description) = token_error(&reply);
        return Err(match error.as_str() {
            "invalid_client" => SpotifyError::InvalidClient {
                redirect_uri: redirect_uri.into(),
            },
            "invalid_grant" if description.to_lowercase().contains("redirect") => {
                SpotifyError::InvalidClient {
                    redirect_uri: redirect_uri.into(),
                }
            }
            "invalid_grant" => SpotifyError::LoginExpired,
            _ => token_http_error(&reply, &description),
        });
    }
    token_from(&reply, None, now_secs)
}

pub(crate) fn refresh(
    http: &Http,
    accounts_base: &str,
    client_id: &str,
    refresh_token: &str,
    now_secs: u64,
) -> Result<Token> {
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ];
    let reply = post_token(http, accounts_base, &form)?;
    if reply.status != 200 {
        let (error, description) = token_error(&reply);
        return Err(match error.as_str() {
            "invalid_grant" | "invalid_client" => SpotifyError::Revoked,
            _ => token_http_error(&reply, &description),
        });
    }
    token_from(&reply, Some(refresh_token), now_secs)
}

fn post_token(http: &Http, accounts_base: &str, form: &[(&str, &str)]) -> Result<Reply> {
    http.send(
        Method::Post,
        &format!("{accounts_base}/api/token"),
        &[],
        None,
        Body::Form(form),
    )
}

fn token_from(reply: &Reply, old_refresh: Option<&str>, now_secs: u64) -> Result<Token> {
    let json: Value =
        serde_json::from_str(&reply.body).map_err(|e| SpotifyError::Parse(e.to_string()))?;
    let access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| SpotifyError::Parse("token response without access_token".into()))?;
    let refresh_token = json["refresh_token"]
        .as_str()
        .or(old_refresh)
        .ok_or_else(|| SpotifyError::Parse("token response without refresh_token".into()))?;
    Ok(Token {
        access_token: access_token.into(),
        refresh_token: refresh_token.into(),
        expires_at: now_secs + json["expires_in"].as_u64().unwrap_or(3600),
        scope: json["scope"].as_str().unwrap_or("").into(),
    })
}

/// `{"error":"invalid_grant","error_description":"…"}` → (error, description).
fn token_error(reply: &Reply) -> (String, String) {
    let json: Value = serde_json::from_str(&reply.body).unwrap_or(Value::Null);
    (
        json["error"].as_str().unwrap_or("").into(),
        json["error_description"].as_str().unwrap_or("").into(),
    )
}

fn token_http_error(reply: &Reply, description: &str) -> SpotifyError {
    if reply.status == 429 {
        return SpotifyError::RateLimited {
            retry_after_secs: reply.retry_after.unwrap_or(5),
        };
    }
    SpotifyError::Http {
        status: reply.status,
        message: description.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::test_server::{Response, serve};

    #[test]
    fn authorize_url_carries_every_parameter() {
        let url = authorize_url(
            "https://accounts.spotify.com",
            "cid",
            "http://127.0.0.1:8898/callback",
            "chal",
            "st",
        );
        assert!(
            url.starts_with("https://accounts.spotify.com/authorize?"),
            "{url}"
        );
        assert!(url.contains("client_id=cid"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A8898%2Fcallback"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("code_challenge=chal"));
        assert!(url.contains("state=st"));
        assert!(url.contains("scope=user-read-private%20user-read-playback-state"));
    }

    #[test]
    fn exchanges_a_code() {
        let (origin, seen) = serve(|_| {
            Response::json(
                200,
                r#"{"access_token":"a1","token_type":"Bearer","expires_in":3600,"refresh_token":"r1","scope":"s"}"#,
            )
        });
        let token = exchange_code(
            &Http::new(),
            &origin,
            "cid",
            "code1",
            "verifier1",
            "http://127.0.0.1:8898/callback",
            1_000,
        )
        .unwrap();
        assert_eq!(token.access_token, "a1");
        assert_eq!(token.refresh_token, "r1");
        assert_eq!(token.expires_at, 4_600);
        let request = seen.lock().unwrap()[0].clone();
        assert_eq!(request.method, "POST");
        assert_eq!(request.path(), "/api/token");
        for part in [
            "grant_type=authorization_code",
            "code=code1",
            "code_verifier=verifier1",
            "client_id=cid",
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A8898%2Fcallback",
        ] {
            assert!(request.body.contains(part), "{part} in {}", request.body);
        }
    }

    #[test]
    fn refresh_keeps_the_old_refresh_token_when_none_comes_back() {
        let (origin, seen) =
            serve(|_| Response::json(200, r#"{"access_token":"a2","expires_in":60}"#));
        let token = refresh(&Http::new(), &origin, "cid", "r-old", 10).unwrap();
        assert_eq!(token.access_token, "a2");
        assert_eq!(token.refresh_token, "r-old");
        assert_eq!(token.expires_at, 70);
        let body = seen.lock().unwrap()[0].body.clone();
        assert!(body.contains("grant_type=refresh_token"), "{body}");
        assert!(body.contains("refresh_token=r-old"), "{body}");
    }

    #[test]
    fn token_errors_become_spotify_errors() {
        let (origin, _) = serve(|request| {
            if request.body.contains("client_id=bad") {
                Response::json(
                    400,
                    r#"{"error":"invalid_client","error_description":"Invalid client"}"#,
                )
            } else if request.body.contains("grant_type=refresh_token") {
                Response::json(
                    400,
                    r#"{"error":"invalid_grant","error_description":"Refresh token revoked"}"#,
                )
            } else {
                Response::json(
                    400,
                    r#"{"error":"invalid_grant","error_description":"Invalid redirect URI"}"#,
                )
            }
        });
        let http = Http::new();
        let uri = "http://127.0.0.1:8898/callback";
        assert_eq!(
            exchange_code(&http, &origin, "bad", "c", "v", uri, 0).unwrap_err(),
            SpotifyError::InvalidClient {
                redirect_uri: uri.into()
            }
        );
        assert_eq!(
            exchange_code(&http, &origin, "cid", "c", "v", uri, 0).unwrap_err(),
            SpotifyError::InvalidClient {
                redirect_uri: uri.into()
            }
        );
        assert_eq!(
            refresh(&http, &origin, "cid", "r", 0).unwrap_err(),
            SpotifyError::Revoked
        );
    }
}
