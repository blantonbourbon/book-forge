mod api;
mod errors;
mod fetch;
mod jobs;

use axum::Router;

use jobs::AppState;

pub use fetch::SharedFetcher;

pub fn boundary_name() -> &'static str {
    "backend"
}

pub fn app() -> Router {
    app_with_fetcher(SharedFetcher::fixture_or_http())
}

pub fn app_with_fetcher(fetcher: SharedFetcher) -> Router {
    api::router(AppState::new(fetcher))
}

pub mod test_support {
    use std::time::Duration;

    use axum::Router;

    use crate::{app_with_fetcher, fetch::SharedFetcher};

    pub fn fixture_app() -> Router {
        app_with_fetcher(SharedFetcher::fixture_or_http())
    }

    pub fn delayed_fixture_app(delay: Duration) -> Router {
        app_with_fetcher(SharedFetcher::fixture_or_http_with_delay(delay))
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
