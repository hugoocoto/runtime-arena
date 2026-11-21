//! What the user sees when a command fails, and its exit code.

use anyhow::Error;
use reqwest::StatusCode;

use crate::client::{Client, Failure};

/// The command line was wrong or incomplete (also clap's own exit code).
pub const EXIT_USAGE: u8 = 2;
/// The server could not be reached.
pub const EXIT_NETWORK: u8 = 3;
/// Not logged in, or the server refused the credentials.
pub const EXIT_AUTH: u8 = 4;
/// The server rejected the request (other 4XX).
pub const EXIT_REJECTED: u8 = 5;
/// The server failed (5XX) or sent something unreadable.
pub const EXIT_SERVER: u8 = 6;
/// Anything else, such as an unreadable local file.
pub const EXIT_OTHER: u8 = 7;

/// A missing or invalid argument found after clap's parsing.
#[derive(Debug)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

/// The command needs a login that isn't there.
#[derive(Debug)]
pub struct NotLoggedIn(pub String);

impl std::fmt::Display for NotLoggedIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotLoggedIn {}

fn find<T: std::error::Error + 'static>(e: &Error) -> Option<&T> {
    e.chain().find_map(|cause| cause.downcast_ref::<T>())
}

/// The HTTP status of a failed request, if that is what `e` is.
pub fn status_of(e: &Error) -> Option<StatusCode> {
    match find::<Failure>(e)? {
        Failure::Rejected { status, .. } => Some(*status),
        _ => None,
    }
}

pub fn exit_code(e: &Error) -> u8 {
    if find::<UsageError>(e).is_some() {
        return EXIT_USAGE;
    }
    if find::<NotLoggedIn>(e).is_some() {
        return EXIT_AUTH;
    }
    match find::<Failure>(e) {
        Some(Failure::Unreachable { .. }) => EXIT_NETWORK,
        Some(Failure::BadResponse { .. }) => EXIT_SERVER,
        Some(Failure::Rejected { status, .. }) => match *status {
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => EXIT_AUTH,
            s if s.is_client_error() => EXIT_REJECTED,
            _ => EXIT_SERVER,
        },
        None => EXIT_OTHER,
    }
}

/// What to do next, when the error alone doesn't say.
fn hint(e: &Error) -> Option<String> {
    if find::<UsageError>(e).is_some() {
        return Some("For more information, try '--help'.".to_owned());
    }
    match find::<Failure>(e)? {
        Failure::Unreachable { server, .. } => Some(format!(
            "Check that the server is running and the address is right \
             (change it with `arena login <server>`). Current address: {server}"
        )),
        Failure::Rejected {
            status,
            path,
            server,
            ..
        } if *status == StatusCode::UNAUTHORIZED => Some(if path == "/api/login" {
            format!("No account yet? Create one with `arena signup {server}`.")
        } else if path.starts_with("/api/admin") {
            "The admin token was refused. Check it and run `arena login --admin`.".to_owned()
        } else {
            "Your session is no longer valid. Run `arena login`.".to_owned()
        }),
        _ => None,
    }
}

/// Whether different CLI and server versions could explain this error.
fn may_be_version_mismatch(e: &Error) -> bool {
    match find::<Failure>(e) {
        Some(Failure::BadResponse { .. }) => true,
        Some(Failure::Rejected { status, .. }) => matches!(
            *status,
            StatusCode::BAD_REQUEST
                | StatusCode::NOT_FOUND
                | StatusCode::METHOD_NOT_ALLOWED
                | StatusCode::UNSUPPORTED_MEDIA_TYPE
                | StatusCode::UNPROCESSABLE_ENTITY
        ),
        _ => false,
    }
}

/// Prints the error with its hint, and returns the exit code.
pub async fn report(e: &Error) -> u8 {
    eprintln!("error: {e:#}");
    if let Some(hint) = hint(e) {
        eprintln!("\n{hint}");
    }
    if may_be_version_mismatch(e)
        && let Some(Failure::Rejected { server, .. } | Failure::BadResponse { server }) =
            find::<Failure>(e)
        && let Ok(client) = Client::new(server, None)
    {
        client.warn_if_version_differs().await;
    }
    exit_code(e)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejected(status: StatusCode, path: &str) -> Error {
        Failure::Rejected {
            status,
            message: "nope".into(),
            path: path.into(),
            server: "http://a:8080".into(),
        }
        .into()
    }

    #[test]
    fn exit_codes_tell_failures_apart() {
        let code = |s, p| exit_code(&rejected(s, p));
        assert_eq!(code(StatusCode::UNAUTHORIZED, "/api/me"), EXIT_AUTH);
        assert_eq!(code(StatusCode::FORBIDDEN, "/api/signup"), EXIT_AUTH);
        assert_eq!(code(StatusCode::NOT_FOUND, "/api/x"), EXIT_REJECTED);
        assert_eq!(code(StatusCode::TOO_MANY_REQUESTS, "/x"), EXIT_REJECTED);
        assert_eq!(code(StatusCode::BAD_GATEWAY, "/x"), EXIT_SERVER);
        assert_eq!(exit_code(&UsageError("x".into()).into()), EXIT_USAGE);
        assert_eq!(exit_code(&NotLoggedIn("x".into()).into()), EXIT_AUTH);
        assert_eq!(exit_code(&anyhow::anyhow!("x")), EXIT_OTHER);
    }

    #[test]
    fn exit_code_survives_added_context() {
        let e = rejected(StatusCode::UNAUTHORIZED, "/api/admin/users").context("verifying");
        assert_eq!(exit_code(&e), EXIT_AUTH);
    }

    #[test]
    fn a_401_hint_depends_on_what_was_refused() {
        let text = |p| hint(&rejected(StatusCode::UNAUTHORIZED, p)).unwrap();
        assert!(text("/api/login").contains("arena signup http://a:8080"));
        assert!(text("/api/admin/users").contains("login --admin"));
        assert!(text("/api/me").contains("arena login"));
        assert!(hint(&rejected(StatusCode::NOT_FOUND, "/api/me")).is_none());
    }
}
