use url::Url;

pub(crate) fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host_str() == right.host_str()
        && left.port_or_known_default() == right.port_or_known_default()
}

pub(crate) fn normalize_page_url(url: &Url) -> String {
    let mut normalized = url.clone();
    normalized.set_fragment(None);
    normalize_default_page_path(&mut normalized);
    normalized.to_string()
}

pub(crate) fn normalize_resource_url(url: &Url) -> String {
    let mut normalized = url.clone();
    normalized.set_fragment(None);
    normalized.to_string()
}

pub(crate) fn url_without_fragment(url: &Url) -> String {
    let mut normalized = url.clone();
    normalized.set_fragment(None);
    normalized.to_string()
}

pub(crate) fn default_prefix_for(start_url: &Url) -> String {
    let mut prefix = start_url.clone();
    prefix.set_query(None);
    prefix.set_fragment(None);

    let path = prefix.path();
    let directory = if path.ends_with('/') {
        path.to_string()
    } else {
        match path.rsplit_once('/') {
            Some(("", _)) => "/".to_string(),
            Some((directory, _)) => format!("{directory}/"),
            None => "/".to_string(),
        }
    };

    prefix.set_path(&directory);
    prefix.to_string()
}

fn normalize_default_page_path(url: &mut Url) {
    let path = url.path();
    let mut normalized = path.to_string();

    for suffix in ["/index.html", "/index.htm"] {
        if normalized.ends_with(suffix) {
            let keep = normalized.len() - suffix.len() + 1;
            normalized.truncate(keep);
            break;
        }
    }

    if normalized.is_empty() {
        normalized.push('/');
    }

    if !normalized.ends_with('/') {
        let last_segment = normalized.rsplit('/').next().unwrap_or_default();
        if !last_segment.contains('.') {
            normalized.push('/');
        }
    }

    url.set_path(&normalized);
}
