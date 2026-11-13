//! Which service address the hosted page may send AI and account requests to.
//!
//! The page's content security policy names one origin, set when the site is built (`LF_CLOUD_URL`); a
//! request to anywhere else would be blocked by the browser anyway, so the page refuses it first, with a
//! reason a person can read.

use laserfoundry_ai::services::Services;

/// The origin (`https://host[:port]`) of `url`, if it has one.
pub fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = match url.strip_prefix("https://") {
        Some(rest) => ("https", rest),
        // plain HTTP only to this computer, for trying the service locally
        None => ("http", url.strip_prefix("http://")?),
    };
    let host = rest.split(['/', '?', '#']).next()?;
    let name = host.rsplit_once(':').filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit())).map_or(host, |(name, _)| name);
    let local = matches!(name, "127.0.0.1" | "localhost" | "[::1]");
    let clean = !host.is_empty() && !host.chars().any(|c| c.is_whitespace() || c.is_control() || c == '@' || c == '\\');
    (clean && (scheme == "https" || local)).then(|| format!("{scheme}://{host}"))
}

/// The services for this browser tab: `?services=staging` asks for staging (and is remembered for the tab's session),
/// `?services=production` goes back to the real ones, and any other value changes nothing. Returns the mode and whether
/// the tab should remember staging (`Some(true)`), forget it (`Some(false)`) or leave its memory alone (`None`).
pub fn choose_services(query: Option<&str>, remembered: Option<&str>) -> (Services, Option<bool>) {
    match query {
        Some("staging") => (Services::Staging, Some(true)),
        Some("production") => (Services::Production, Some(false)),
        _ => (Services::requested(remembered.unwrap_or_default()), None),
    }
}

/// Whether a request to `url` may go out from a page built for the service at `built_in` (empty or absent
/// means the page was built with none).
pub fn allowed(url: &str, built_in: Option<&str>) -> bool {
    match (origin(url), built_in.and_then(origin)) {
        (Some(wanted), Some(allowed)) => wanted == allowed,
        _ => false,
    }
}

/// Whether the page may POST `url`: the service it was built for, or MakerFoundry's address for sending a design
/// (the page's content security policy names MakerFoundry for this and for [`fetch_allowed`]).
pub fn post_allowed(url: &str, built_in: Option<&str>, services: Services) -> bool {
    match services {
        // staging talks to its own pair only, never to the real services
        Services::Staging => allowed(url, Some(services.cloud())) || url == format!("{}/api/v1/designs", services.maker()),
        Services::Production => allowed(url, built_in) || url == format!("{}/api/v1/designs", services.maker()),
    }
}

/// Whether the page may GET `url`: a design file on one of MakerFoundry's own hosts, exactly as the app checked it.
pub fn fetch_allowed(url: &str, services: Services) -> bool {
    laserfoundry_ai::maker::open_link_in(services, url).is_ok_and(|link| link.url == url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makerfoundry_may_be_posted_a_design_and_asked_for_design_files_and_nothing_else() {
        let b = Some("https://cloud.example.com");
        assert!(post_allowed("https://makerfoundry.app/api/v1/designs", b, Services::Production));
        assert!(post_allowed("https://cloud.example.com/v1/messages", b, Services::Production));
        for bad in [
            "https://makerfoundry.app/api/v1/account",
            "https://makerfoundry.app/api/v1/designs/1",
            "https://makerfoundry.app/api/v1/designs?x=1",
            "http://makerfoundry.app/api/v1/designs",
            "https://www.makerfoundry.app/api/v1/designs",
            "https://makerfoundry.app.evil.com/api/v1/designs",
            "https://makerfoundry.app@evil.com/api/v1/designs",
            "https://evil.com/api/v1/designs",
            "",
        ] {
            assert!(!post_allowed(bad, b, Services::Production), "{bad}");
        }
        assert!(fetch_allowed("https://makerfoundry.app/api/v1/designs/1/file/a.svg", Services::Production));
        assert!(fetch_allowed("https://www.makerfoundry.app/api/v1/designs/1/file/a.svg?t=x", Services::Production));
        for bad in [
            "https://cloud.example.com/v1/account",
            "https://makerfoundry.app/login",
            "https://makerfoundry.app:8443/api/v1/designs/1/file/a.svg",
            "https://makerfoundry.app.evil.com/api/v1/designs/1/file/a.svg",
            "https://MakerFoundry.app/api/v1/designs/1/file/a.svg",
            " https://makerfoundry.app/api/v1/designs/1/file/a.svg",
            "https://makerfoundry.app/api/v1/designs/1/file/a.svg#frag",
            "",
        ] {
            assert!(!fetch_allowed(bad, Services::Production), "{bad:?}: the host fetches only what the app checked, unchanged");
        }
    }

    #[test]
    fn services_come_from_the_exact_query_word_and_are_remembered_for_the_tab() {
        assert_eq!(choose_services(None, None), (Services::Production, None), "production is the default");
        assert_eq!(choose_services(Some("staging"), None), (Services::Staging, Some(true)));
        assert_eq!(choose_services(None, Some("staging")), (Services::Staging, None), "a reload in the same tab stays on staging");
        assert_eq!(choose_services(Some("production"), Some("staging")), (Services::Production, Some(false)));
        for bad in ["", "Staging", "https://cloud-staging-staging.up.railway.app", "staging.evil.com", "stagin", "1"] {
            assert_eq!(choose_services(Some(bad), None), (Services::Production, None), "{bad:?} changes nothing");
            assert_eq!(choose_services(Some(bad), Some("staging")).0, Services::Staging, "{bad:?} does not switch an earlier choice off either");
        }
        assert_eq!(choose_services(None, Some("junk")), (Services::Production, None), "only the exact word is remembered");
    }

    #[test]
    fn staging_reaches_only_the_staging_pair_and_production_never_reaches_them() {
        let b = Some("https://cloud.example.com");
        let (cloud, maker) = (laserfoundry_ai::services::STAGING_CLOUD, laserfoundry_ai::services::STAGING_MAKER);
        assert!(post_allowed(&format!("{cloud}/v1/messages"), b, Services::Staging));
        assert!(post_allowed(&format!("{maker}/api/v1/designs"), b, Services::Staging));
        assert!(fetch_allowed(&format!("{maker}/api/v1/designs/3/file/a.svg"), Services::Staging));
        for bad in ["https://cloud.example.com/v1/messages", "https://cloud.stitchfoundry.app/v1/messages", "https://makerfoundry.app/api/v1/designs"]
        {
            assert!(!post_allowed(bad, b, Services::Staging), "{bad}: staging must never create test data on the real services");
        }
        assert!(!fetch_allowed("https://makerfoundry.app/api/v1/designs/1/file/a.svg", Services::Staging));
        for bad in [format!("{cloud}/v1/messages"), format!("{maker}/api/v1/designs")] {
            assert!(!post_allowed(&bad, b, Services::Production), "{bad}");
        }
        assert!(!fetch_allowed(&format!("{maker}/api/v1/designs/3/file/a.svg"), Services::Production));
    }

    #[test]
    fn only_the_built_in_origin_is_allowed() {
        let b = Some("https://cloud.example.com/");
        assert!(allowed("https://cloud.example.com/v1/messages", b));
        assert!(allowed("https://cloud.example.com", b));
        for bad in [
            "https://evil.example.com/v1/messages",
            "https://cloud.example.com.evil.com/x",
            "https://cloud.example.com@evil.com/x",
            "http://cloud.example.com/x",
            "http://127.0.0.1.evil.com/x",
            "https://cloud.example.com:8443/x",
            "//cloud.example.com/x",
            "https://",
            "",
        ] {
            assert!(!allowed(bad, b), "{bad}");
        }
        assert!(
            !allowed("https://cloud.example.com/x", None) && !allowed("https://cloud.example.com/x", Some("")),
            "no built-in service, none allowed"
        );
    }

    #[test]
    fn origins_are_read_strictly() {
        assert_eq!(origin("https://a.b:8443/x?y#z").as_deref(), Some("https://a.b:8443"));
        assert_eq!(origin("https://a b/x"), None);
        assert_eq!(origin("ftp://a.b"), None);
        assert_eq!(origin("http://127.0.0.1:8766/v1/x").as_deref(), Some("http://127.0.0.1:8766"));
        assert_eq!(origin("http://localhost/x").as_deref(), Some("http://localhost"));
        assert_eq!(origin("http://example.com"), None, "plain HTTP only to this computer");
    }
}
