mod api;
mod errors;
mod fetch;
mod jobs;
mod security;
mod static_files;

use std::{env, path::PathBuf};

use axum::Router;

use jobs::AppState;

pub use fetch::SharedFetcher;

pub fn boundary_name() -> &'static str {
    "backend"
}

pub fn app() -> Router {
    let fetcher = SharedFetcher::fixture_or_http();
    if let Ok(static_dir) = env::var("STATIC_DIR") {
        app_with_fetcher_and_static_dir(fetcher, PathBuf::from(static_dir))
    } else {
        app_with_fetcher(fetcher)
    }
}

pub fn app_with_fetcher(fetcher: SharedFetcher) -> Router {
    api::router(AppState::new(fetcher))
}

pub fn app_with_fetcher_and_static_dir(fetcher: SharedFetcher, static_dir: PathBuf) -> Router {
    api::router(AppState::new(fetcher).with_static_root(static_dir))
}

pub mod test_support {
    use std::{net::SocketAddr, path::PathBuf, time::Duration};

    use axum::Router;

    use crate::{app_with_fetcher, app_with_fetcher_and_static_dir, fetch::SharedFetcher};

    pub fn fixture_app() -> Router {
        app_with_fetcher(SharedFetcher::fixture_or_http())
    }

    pub fn delayed_fixture_app(delay: Duration) -> Router {
        app_with_fetcher(SharedFetcher::fixture_or_http_with_delay(delay))
    }

    pub fn resolved_host_fixture_app(domain: &str, addrs: &[SocketAddr]) -> Router {
        app_with_fetcher(SharedFetcher::fixture_or_http_with_resolved_host(
            domain, addrs,
        ))
    }

    pub fn static_fixture_app(static_dir: PathBuf) -> Router {
        app_with_fetcher_and_static_dir(SharedFetcher::fixture_or_http(), static_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::boundary_name;

    #[test]
    fn exposes_backend_boundary_name() {
        assert_eq!(boundary_name(), "backend");
    }
}
