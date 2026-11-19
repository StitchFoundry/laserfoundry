#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use laserfoundry_device::Transport;
use laserfoundry_engine::Session;

use super::*;
use crate::Platform;

mod offer_tests {
    use super::*;

    #[test]
    fn the_first_export_of_a_project_opens_the_prompt_and_later_ones_do_not() {
        let (mut app, _) = app(true, HashMap::new());
        app.offer_share();
        assert!(app.maker.share.open);
        app.finish_share(false);
        app.offer_share();
        assert!(!app.maker.share.open, "once per project");
    }

    #[test]
    fn another_project_is_asked_again() {
        let (mut app, _) = app(true, HashMap::new());
        app.offer_share();
        app.finish_share(false);
        app.session.path = Some(PathBuf::from("other.laserfoundry"));
        app.offer_share();
        assert!(app.maker.share.open);
    }

    #[test]
    fn the_memory_survives_a_restart() {
        let (mut first, store) = app(true, HashMap::new());
        first.offer_share();
        let kept = store.borrow().clone();
        let (mut second, _) = app(true, kept);
        second.offer_share();
        assert!(!second.maker.share.open);
    }

    #[test]
    fn the_setting_off_means_no_prompt() {
        let (mut app, _) = app(true, HashMap::from([(OFFER_KEY.to_string(), "off".to_string())]));
        app.offer_share();
        assert!(!app.maker.share.open);
    }

    #[test]
    fn a_host_without_a_network_never_asks() {
        let (mut app, _) = app(false, HashMap::new());
        app.offer_share();
        assert!(!app.maker.share.open);
    }

    #[test]
    fn the_memory_is_capped() {
        let (mut app, _) = app(true, HashMap::new());
        for i in 0..MAX_ASKED + 20 {
            app.session.path = Some(PathBuf::from(format!("p{i}.laserfoundry")));
            app.offer_share();
            app.finish_share(false);
        }
        assert_eq!(app.maker.share.asked.len(), MAX_ASKED);
    }
}

mod choice_tests {
    use super::*;

    #[test]
    fn not_now_closes_and_opens_nothing() {
        let (mut app, _) = app(true, HashMap::new());
        app.offer_share();
        app.finish_share(false);
        assert!(!app.maker.share.open);
        assert!(app.pending_url.is_none());
        assert!(app.share_offer_on(), "not now keeps the offer on");
    }

    #[test]
    fn share_opens_makerfoundry_without_touching_the_account() {
        let (mut app, _) = app(true, HashMap::new());
        app.offer_share();
        app.finish_share(true);
        assert_eq!(app.pending_url.as_deref(), Some(app.platform.services().maker()));
        assert!(!app.ai.signed_in());
        assert!(app.ai.window.is_none());
    }

    #[test]
    fn dont_ask_again_turns_the_offer_off_and_keeps_it_off() {
        let (mut app, store) = app(true, HashMap::new());
        app.offer_share();
        app.maker.share.dont_ask = true;
        app.finish_share(false);
        assert!(!app.share_offer_on());
        assert_eq!(store.borrow().get(OFFER_KEY).map(String::as_str), Some("off"));
    }

    #[test]
    fn the_settings_switch_turns_it_back_on() {
        let (mut app, _) = app(true, HashMap::from([(OFFER_KEY.to_string(), "off".to_string())]));
        app.set_share_offer(true);
        app.offer_share();
        assert!(app.maker.share.open);
    }

    #[test]
    fn drawing_the_open_prompt_does_not_close_it() {
        let (mut app, _) = app(true, HashMap::new());
        app.offer_share();
        egui::Context::default().run_ui(egui::RawInput::default(), |ui| app.share_prompt(ui.ctx())).drop_without_applying_deltas();
        assert!(app.maker.share.open, "it waits for a choice and blocks nothing");
    }
}

type Store = Rc<RefCell<HashMap<String, String>>>;

struct Fake {
    http: bool,
    store: Store,
}

impl Platform for Fake {
    fn pick_open(&self, _: &str, _: &[&str]) -> Option<PathBuf> {
        None
    }

    fn pick_save(&self, _: &str, _: &str, _: &[&str]) -> Option<PathBuf> {
        None
    }

    fn serial_ports(&self) -> Vec<String> {
        Vec::new()
    }

    fn open_serial(&self, _: &str) -> Result<Box<dyn Transport + Send>, String> {
        Err("no serial".into())
    }

    fn http_available(&self) -> bool {
        self.http
    }

    fn storage_get(&self, key: &str) -> Option<String> {
        self.store.borrow().get(key).cloned()
    }

    fn storage_set(&self, key: &str, value: &str) {
        self.store.borrow_mut().insert(key.to_string(), value.to_string());
    }
}

fn app(http: bool, stored: HashMap<String, String>) -> (LaserFoundryApp, Store) {
    let store = Rc::new(RefCell::new(stored));
    let mut app = LaserFoundryApp::new(Session::new(), Box::new(Fake { http, store: Rc::clone(&store) }));
    app.session.path = Some(PathBuf::from("rose.laserfoundry"));
    (app, store)
}
