//! Which pair of services (account + MakerFoundry) the app talks to: the real ones, or the staging pair used by
//! automated release tests (`stitch-e2e`), which must never create test accounts or designs on the real services.
//!
//! The staging addresses are compiled in, exact and HTTPS: nothing in a link or a setting can name another address.
//! The only input is a short word (`services=staging`); anything else means the real services.

use super::{cloud, maker};

/// The staging account service.
pub const STAGING_CLOUD: &str = "https://cloud-staging-staging.up.railway.app";
/// The staging MakerFoundry.
pub const STAGING_MAKER: &str = "https://makerfoundry-test-test.up.railway.app";
/// The host of [`STAGING_MAKER`].
pub const STAGING_MAKER_HOST: &str = "makerfoundry-test-test.up.railway.app";
/// What the app shows for as long as it uses the staging pair.
pub const STAGING_BANNER: &str = "STAGING services (test data)";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Services {
    #[default]
    Production,
    Staging,
}

impl Services {
    /// The mode a `services=` value asks for: exactly `staging`. Anything else (other words, other cases, addresses)
    /// is not a request for staging.
    pub fn requested(value: &str) -> Services {
        if value == "staging" { Services::Staging } else { Services::Production }
    }

    /// The account service for this mode (the build's own for the real services).
    pub fn cloud(self) -> &'static str {
        match self {
            Services::Production => cloud::built_in_server(),
            Services::Staging => STAGING_CLOUD,
        }
    }

    /// MakerFoundry for this mode.
    pub fn maker(self) -> &'static str {
        match self {
            Services::Production => maker::DEFAULT_SERVER,
            Services::Staging => STAGING_MAKER,
        }
    }

    /// The hosts a design may be opened from in this mode: staging accepts only its own MakerFoundry.
    pub fn open_hosts(self) -> &'static [&'static str] {
        match self {
            Services::Production => &maker::OPEN_HOSTS,
            Services::Staging => &[STAGING_MAKER_HOST],
        }
    }

    pub fn banner(self) -> Option<&'static str> {
        (self == Services::Staging).then_some(STAGING_BANNER)
    }

    /// Where this mode keeps the account's sign-in on this computer: a separate place for staging, so a real session
    /// token is never sent to the staging service nor a staging one to the real service.
    pub fn secret_key(self, base: &str) -> String {
        match self {
            Services::Production => base.to_string(),
            Services::Staging => format!("{base}.staging"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_word_staging_asks_for_staging() {
        assert_eq!(Services::requested("staging"), Services::Staging);
        for other in ["", "production", "Staging", "STAGING", " staging", "staging ", "https://evil.example", STAGING_CLOUD, "staging,evil", "stagin"]
        {
            assert_eq!(Services::requested(other), Services::Production, "{other:?}");
        }
        assert_eq!(Services::default(), Services::Production);
    }

    #[test]
    fn staging_addresses_are_the_compiled_in_ones_and_production_is_unchanged() {
        assert_eq!(Services::Staging.cloud(), "https://cloud-staging-staging.up.railway.app");
        assert_eq!(Services::Staging.maker(), "https://makerfoundry-test-test.up.railway.app");
        assert_eq!(Services::Production.cloud(), cloud::built_in_server());
        assert_eq!(Services::Production.maker(), "https://makerfoundry.app");
        assert!(STAGING_CLOUD.starts_with("https://") && STAGING_MAKER == format!("https://{STAGING_MAKER_HOST}"));
        assert_eq!(Services::Production.banner(), None);
        assert_eq!(Services::Staging.banner(), Some("STAGING services (test data)"));
    }

    #[test]
    fn each_mode_opens_designs_only_from_its_own_makerfoundry_and_keeps_its_own_sign_in() {
        assert_eq!(Services::Staging.open_hosts(), [STAGING_MAKER_HOST]);
        assert!(Services::Production.open_hosts().contains(&"makerfoundry.app") && !Services::Production.open_hosts().contains(&STAGING_MAKER_HOST));
        assert_ne!(Services::Staging.secret_key("k"), Services::Production.secret_key("k"));
        assert_eq!(Services::Production.secret_key("k"), "k");
    }
}
