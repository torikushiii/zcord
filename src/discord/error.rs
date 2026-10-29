use super::*;

#[derive(Clone, Debug)]
pub enum Error {
    RateLimited {
        retry_after: Duration,
    },
    Http(u16),
    Captcha,
    Rejected {
        status: u16,
        code: u64,
        message: String,
    },
    Network,
    Decode,
    InvalidCredential,
    Closed,
    SettingsOutdated,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RateLimited { retry_after } => write!(
                f,
                "Rate limited; retry after {:.1}s",
                retry_after.as_secs_f64()
            ),
            Self::Http(status) => write!(f, "HTTP {status}"),
            Self::Captcha => f.write_str("Discord requested verification"),
            Self::Rejected {
                status,
                code,
                message,
            } => write!(f, "HTTP {status} ({code}): {message}"),
            Self::Network => f.write_str("Network request failed"),
            Self::Decode => f.write_str("Malformed Discord response"),
            Self::InvalidCredential => f.write_str("Invalid credential header"),
            Self::Closed => f.write_str("Session closed"),
            Self::SettingsOutdated => f.write_str("Status changed on another client; choose again"),
        }
    }
}
impl std::error::Error for Error {}
