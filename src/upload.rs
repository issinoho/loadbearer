//! `run --upload`: post the finished result to a loadbearer-fleet server.
//!
//! The one outbound call loadbearer makes that isn't a link probe, and only
//! when it is asked for. Four decisions shape it:
//!
//! **The credential never goes in the URL or on the command line.** A URL is
//! written down everywhere it passes — proxy and server access logs, the
//! deployment tool's step history, `--log-file` — and a command line shows up
//! in process listings and EDR telemetry. The URL says where; a file (or
//! `LOADBEARER_UPLOAD_TOKEN`) says who. A URL carrying a user, a password or a
//! query string is refused, because that is what a key in the URL looks like.
//!
//! **https, except to this machine.** A bearer token sent in the clear is a
//! token given away. Plain `http://` is allowed to loopback only, which is
//! what a server behind a local TLS proxy, or a test, looks like.
//!
//! **Redirects are not followed.** A redirect is a server asking for the
//! `Authorization` header to be sent somewhere else; the answer is no, and the
//! response says where it wanted to go so the URL can be fixed instead.
//!
//! **A failed upload never loses the result.** It is posted after `--output`
//! is written, as exactly those bytes — so a machine whose file is also
//! collected from a share is indexed once, because the server deduplicates
//! by the hash of the document.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use log::{info, warn};

/// The environment variable read when no token file is given.
pub const TOKEN_ENV: &str = "LOADBEARER_UPLOAD_TOKEN";

/// Attempts in all, counting the first. A rate-limited or briefly
/// unreachable server gets two more chances; a refused token gets none.
const ATTEMPTS: u32 = 3;

/// The longest `Retry-After` honoured. A fleet run happens on a schedule and
/// a deployment step has a timeout; waiting out a long one belongs to the next
/// run, which will find the result on disk under `--output`.
const MAX_WAIT: Duration = Duration::from_secs(120);

/// Where to send, and who is sending. The token is deliberately not `Debug`.
pub struct Target {
    pub url: String,
    token: String,
}

/// What the server said about a delivered result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered {
    Added,
    AlreadyIndexed,
}

/// Work out the upload target from the command line, the config file and the
/// environment — or `None` when no upload was asked for.
///
/// Called before the benchmarks run, so a missing token or a bad URL is found
/// in a second rather than after a five-minute run.
pub fn resolve(
    url: Option<&str>,
    token_file: Option<&Path>,
    file: Option<&crate::config::UploadConfig>,
    env_token: Option<String>,
) -> Result<Option<Target>> {
    let url = match url.or(file.and_then(|f| f.url.as_deref())) {
        Some(url) => url.trim().to_string(),
        None => {
            if token_file.is_some() {
                bail!("--upload-token-file was given without --upload: there is nowhere to send");
            }
            return Ok(None);
        }
    };
    check_url(&url)?;

    let token_file: Option<PathBuf> = token_file
        .map(Path::to_path_buf)
        .or_else(|| file.and_then(|f| f.token_file.clone()));
    let token = match (&token_file, env_token) {
        (Some(path), _) => std::fs::read_to_string(path)
            .with_context(|| format!("reading the upload token from {}", path.display()))?,
        (None, Some(token)) => token,
        (None, None) => bail!(
            "--upload needs a token: pass --upload-token-file FILE, set {TOKEN_ENV}, or set \
             [upload] token_file in the config. `loadbearer-fleet token new` generates one."
        ),
    };
    let token = token.trim().to_string();
    if token.is_empty() {
        bail!(
            "the upload token is empty{}",
            token_file
                .map(|p| format!(" (read from {})", p.display()))
                .unwrap_or_default()
        );
    }
    if token.chars().any(|c| c.is_whitespace() || c.is_control()) {
        bail!(
            "the upload token contains whitespace; the file should hold the token and nothing else"
        );
    }
    Ok(Some(Target { url, token }))
}

/// Refuse a URL that would leak the token, or that has one in it.
fn check_url(url: &str) -> Result<()> {
    let uri: ureq::http::Uri = url
        .parse()
        .with_context(|| format!("--upload {url:?} is not a URL"))?;
    let Some(authority) = uri.authority() else {
        bail!("--upload {url:?} has no host; it wants https://fleet.example.com/api/upload");
    };
    if authority.as_str().contains('@') {
        bail!(
            "--upload {url:?} has a user or password in it. The token goes in \
             --upload-token-file, never in the URL, where every log it passes through keeps it."
        );
    }
    if uri.query().is_some() {
        bail!(
            "--upload {url:?} has a query string. If that is a key, it belongs in \
             --upload-token-file: a URL is written down by every proxy and log it passes."
        );
    }
    match uri.scheme_str() {
        Some("https") => Ok(()),
        Some("http") if is_loopback(uri.host().unwrap_or_default()) => Ok(()),
        Some("http") => bail!(
            "--upload {url:?} is plain http to another machine, which would send the token in \
             the clear. Use the server's https address."
        ),
        _ => bail!("--upload {url:?} must be an https:// URL"),
    }
}

fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The agent every upload goes through.
fn agent() -> ureq::Agent {
    use ureq::tls::{RootCerts, TlsConfig};
    ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(format!("loadbearer/{}", env!("LOADBEARER_VERSION")))
        .tls_config(
            TlsConfig::builder()
                .root_certs(RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into()
}

/// Post one result document, retrying what is worth retrying.
pub fn send(target: &Target, body: &str) -> Result<Delivered> {
    send_with(&agent(), target, body, &std::thread::sleep)
}

/// The same, with the wait injectable so the retries are testable without
/// sleeping through them.
fn send_with(
    agent: &ureq::Agent,
    target: &Target,
    body: &str,
    sleep: &dyn Fn(Duration),
) -> Result<Delivered> {
    let mut attempt = 1;
    loop {
        let reply = post(agent, target, body);
        let wait = match &reply {
            Reply::Status {
                code: 429,
                retry_after,
                ..
            } => Some(retry_after.unwrap_or(Duration::from_secs(5))),
            Reply::Status { code, .. } if *code >= 500 => Some(backoff(attempt)),
            Reply::Unreachable(_) => Some(backoff(attempt)),
            _ => None,
        };
        match (wait, attempt < ATTEMPTS) {
            (Some(wait), true) if wait <= MAX_WAIT => {
                warn!(
                    target: "loadbearer::upload",
                    "attempt {attempt} of {ATTEMPTS} did not land ({}); retrying in {}s",
                    reply.summary(),
                    wait.as_secs()
                );
                sleep(wait);
                attempt += 1;
            }
            _ => return reply.into_outcome(&target.url),
        }
    }
}

/// 2 s, then 8 s.
fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(2 * 4u64.pow(attempt.saturating_sub(1)))
}

enum Reply {
    Status {
        code: u16,
        retry_after: Option<Duration>,
        location: Option<String>,
        text: String,
    },
    Unreachable(String),
}

fn post(agent: &ureq::Agent, target: &Target, body: &str) -> Reply {
    info!(target: "loadbearer::upload", "posting result ({} bytes) to {}", body.len(), target.url);
    let response = agent
        .post(&target.url)
        .header("Authorization", &format!("Bearer {}", target.token))
        .content_type("application/json")
        .send(body);
    match response {
        Ok(mut response) => {
            let header = |name: &str| {
                response
                    .headers()
                    .get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            let retry_after = header("retry-after")
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let location = header("location");
            let code = response.status().as_u16();
            let text = response
                .body_mut()
                .with_config()
                .limit(64 * 1024)
                .read_to_string()
                .unwrap_or_default();
            Reply::Status {
                code,
                retry_after,
                location,
                text,
            }
        }
        Err(e) => Reply::Unreachable(e.to_string()),
    }
}

impl Reply {
    fn summary(&self) -> String {
        match self {
            Self::Status { code, .. } => format!("HTTP {code}"),
            Self::Unreachable(why) => why.clone(),
        }
    }

    /// What a final reply means, said so that the person reading a
    /// deployment tool's log can fix it without looking anything up.
    fn into_outcome(self, url: &str) -> Result<Delivered> {
        let (code, location, text) = match self {
            Self::Unreachable(why) => bail!(
                "could not reach {url}: {why}. If this machine reaches the web through a proxy, \
                 set HTTPS_PROXY; if the server uses a private CA, it must be trusted by the OS."
            ),
            Self::Status {
                code,
                location,
                text,
                ..
            } => (code, location, text),
        };
        let said = server_said(&text);
        match code {
            200 => {
                let outcome = serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|v| v["outcome"].as_str().map(str::to_string));
                match outcome.as_deref() {
                    Some("added") => Ok(Delivered::Added),
                    Some("already indexed") => Ok(Delivered::AlreadyIndexed),
                    _ => bail!(
                        "{url} answered 200 but not as a loadbearer-fleet upload endpoint would \
                         ({said}); check the URL ends in /api/upload"
                    ),
                }
            }
            300..=399 => bail!(
                "{url} redirected to {}; not followed, because that would send the token there \
                 too. Use the address it redirects to as --upload.",
                location.as_deref().unwrap_or("somewhere unstated")
            ),
            401 => bail!(
                "the server refused the upload token: {said}. Check the token file holds the \
                 token you were issued, and that it has not been revoked or expired."
            ),
            // loadbearer-fleet already names `--tag` in its 403; an older or
            // different server may not, and then the hint is the diagnosis.
            403 if said.contains("--tag") => bail!("the server refused this result: {said}"),
            403 => bail!(
                "the server refused this result: {said}. A token scoped to a site only accepts \
                 results carrying that site's tags — run with the --tag it was issued for."
            ),
            404 | 405 => bail!(
                "{url} has no upload endpoint (HTTP {code}); the URL wants to end in /api/upload"
            ),
            413 => bail!("the server refused the result as too large (HTTP 413): {said}"),
            429 => bail!(
                "the server is rate-limiting this token and asked for longer than this run will \
                 wait: {said}. The result is in --output if one was given; the next run will \
                 deliver it."
            ),
            _ => bail!("the server refused the upload (HTTP {code}): {said}"),
        }
    }
}

/// The server's own explanation: the `error` field of a JSON body, or the
/// start of a text one. Without its closing full stop, because it is always
/// quoted mid-sentence.
fn server_said(text: &str) -> String {
    let said = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| text.trim().to_string());
    if said.is_empty() {
        return "no reason given".to_string();
    }
    let mut short: String = said.chars().take(300).collect();
    if short.len() < said.len() {
        short.push('…');
    }
    short.trim_end_matches('.').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn target(url: &str) -> Target {
        Target {
            url: url.to_string(),
            token: "lbf_s3cret".to_string(),
        }
    }

    #[test]
    fn a_url_that_would_carry_or_leak_the_token_is_refused() {
        for (url, want) in [
            (
                "https://user:pw@fleet.example/api/upload",
                "user or password",
            ),
            (
                "https://lbf_key@fleet.example/api/upload",
                "user or password",
            ),
            ("https://fleet.example/api/upload?key=lbf_x", "query string"),
            ("http://fleet.example/api/upload", "in the clear"),
            ("ftp://fleet.example/api/upload", "https://"),
            ("/api/upload", "no host"),
        ] {
            let err = check_url(url).expect_err(url).to_string();
            assert!(err.contains(want), "{url}: {err}");
        }
        for url in [
            "https://fleet.example/api/upload",
            "https://fleet.example:8443/api/upload",
            "http://127.0.0.1:8787/api/upload",
            "http://localhost:8787/api/upload",
            "http://[::1]:8787/api/upload",
        ] {
            check_url(url).unwrap_or_else(|e| panic!("{url}: {e:#}"));
        }
    }

    #[test]
    fn the_token_comes_from_the_flag_then_the_config_then_the_environment() {
        let dir = std::env::temp_dir().join(format!("lb-upload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let flag = dir.join("flag");
        let file = dir.join("file");
        std::fs::write(&flag, "lbf_from_flag\r\n").unwrap();
        std::fs::write(&file, "lbf_from_config").unwrap();
        let cfg = crate::config::UploadConfig {
            url: Some("https://fleet.example/api/upload".into()),
            token_file: Some(file.clone()),
        };
        let env = || Some("lbf_from_env".to_string());

        let t = resolve(None, Some(&flag), Some(&cfg), env())
            .unwrap()
            .unwrap();
        assert_eq!(t.token, "lbf_from_flag", "trailing newline trimmed");
        assert_eq!(t.url, "https://fleet.example/api/upload");
        let t = resolve(None, None, Some(&cfg), env()).unwrap().unwrap();
        assert_eq!(t.token, "lbf_from_config");
        let t = resolve(Some("https://other.example/api/upload"), None, None, env())
            .unwrap()
            .unwrap();
        assert_eq!(t.token, "lbf_from_env");
        assert_eq!(t.url, "https://other.example/api/upload", "the flag wins");

        let err = resolve(Some("https://fleet.example/api/upload"), None, None, None)
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("needs a token"), "{err}");
        assert!(
            resolve(None, None, None, env()).unwrap().is_none(),
            "no upload asked for"
        );
        assert!(
            resolve(None, Some(&flag), None, None).is_err(),
            "a token file with nowhere to send it is a mistake worth naming"
        );

        std::fs::write(&flag, "  \n").unwrap();
        let err = resolve(
            Some("https://fleet.example/api/upload"),
            Some(&flag),
            None,
            None,
        )
        .err()
        .unwrap()
        .to_string();
        assert!(err.contains("empty"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// One canned reply per connection, in order, recording each request.
    fn server(replies: Vec<String>) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/upload", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        std::thread::spawn(move || {
            for reply in replies {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap_or(0);
                    }
                    head.push_str(&line);
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                log.lock()
                    .unwrap()
                    .push(format!("{head}\r\n{}", String::from_utf8_lossy(&body)));
                let mut stream = stream;
                stream.write_all(reply.as_bytes()).unwrap();
            }
        });
        (url, seen)
    }

    fn reply(status: &str, headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
            body.len()
        )
    }

    const ADDED: &str = r#"{"outcome":"added","machine_key":"K","hostname":"PC-01"}"#;

    #[test]
    fn a_result_is_posted_with_the_token_as_a_bearer_header() {
        let (url, seen) = server(vec![reply("200 OK", "", ADDED)]);
        let outcome = send_with(&agent(), &target(&url), r#"{"schema":"x"}"#, &|_| {}).unwrap();
        assert_eq!(outcome, Delivered::Added);

        let seen = seen.lock().unwrap();
        let request = seen[0].to_ascii_lowercase();
        assert!(request.starts_with("post /api/upload "), "{request}");
        assert!(
            request.contains("authorization: bearer lbf_s3cret"),
            "{request}"
        );
        assert!(
            request.contains("content-type: application/json"),
            "{request}"
        );
        assert!(
            seen[0].ends_with(r#"{"schema":"x"}"#),
            "the exact bytes: {}",
            seen[0]
        );
    }

    #[test]
    fn a_rate_limit_is_waited_out_as_the_server_asks() {
        let (url, seen) = server(vec![
            reply("429 Too Many Requests", "Retry-After: 7\r\n", "slow down"),
            reply("200 OK", "", r#"{"outcome":"already indexed"}"#),
        ]);
        let waited = Mutex::new(Vec::new());
        let outcome = send_with(&agent(), &target(&url), "{}", &|d| {
            waited.lock().unwrap().push(d)
        })
        .unwrap();
        assert_eq!(outcome, Delivered::AlreadyIndexed);
        assert_eq!(*waited.lock().unwrap(), [Duration::from_secs(7)]);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_refused_token_is_not_retried_and_says_what_the_server_said() {
        let (url, seen) = server(vec![reply(
            "401 Unauthorized",
            "Content-Type: application/json\r\n",
            r#"{"error":"that ingest token expired after 2026-06-30; ask for a new one"}"#,
        )]);
        let err = send_with(&agent(), &target(&url), "{}", &|_| panic!("no retry"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("expired after 2026-06-30"), "{err}");
        assert!(
            !err.contains("lbf_s3cret"),
            "the token is never repeated back: {err}"
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_redirect_is_reported_rather_than_followed() {
        let (url, seen) = server(vec![
            reply(
                "302 Found",
                "Location: https://elsewhere.example/api/upload\r\n",
                "",
            ),
            reply("200 OK", "", ADDED),
        ]);
        let err = send_with(&agent(), &target(&url), "{}", &|_| {})
            .unwrap_err()
            .to_string();
        assert!(err.contains("elsewhere.example"), "{err}");
        assert_eq!(
            seen.lock().unwrap().len(),
            1,
            "the redirect was not followed"
        );
    }

    #[test]
    fn a_scope_refusal_points_at_the_tags() {
        let (url, _) = server(vec![reply(
            "403 Forbidden",
            "",
            "this result is not tagged as one you may submit",
        )]);
        let err = send_with(&agent(), &target(&url), "{}", &|_| {})
            .unwrap_err()
            .to_string();
        assert!(err.contains("--tag"), "{err}");

        // A server that already says so is not said over.
        let (url, _) = server(vec![reply(
            "403 Forbidden",
            "",
            "not tagged as one you may submit — pass them with --tag.",
        )]);
        let err = send_with(&agent(), &target(&url), "{}", &|_| {})
            .unwrap_err()
            .to_string();
        assert_eq!(err.matches("--tag").count(), 1, "{err}");
        assert!(!err.contains(".."), "{err}");
    }

    #[test]
    fn a_server_error_is_retried_then_given_up_on() {
        let (url, seen) = server(vec![
            reply("503 Service Unavailable", "", ""),
            reply("502 Bad Gateway", "", ""),
            reply("500 Internal Server Error", "", "disk full"),
        ]);
        let waited = Mutex::new(Vec::new());
        let err = send_with(&agent(), &target(&url), "{}", &|d| {
            waited.lock().unwrap().push(d)
        })
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("HTTP 500") && err.contains("disk full"),
            "{err}"
        );
        assert_eq!(
            *waited.lock().unwrap(),
            [Duration::from_secs(2), Duration::from_secs(8)]
        );
        assert_eq!(seen.lock().unwrap().len(), ATTEMPTS as usize);
    }

    #[test]
    fn a_server_that_cannot_be_reached_says_what_to_check() {
        // Bound then dropped: nothing is listening there any more.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let err = send_with(
            &agent(),
            &target(&format!("http://127.0.0.1:{port}/api/upload")),
            "{}",
            &|_| {},
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("could not reach") && err.contains("HTTPS_PROXY"),
            "{err}"
        );
    }
}
