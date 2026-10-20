//! Keeps the computer awake while agents work (17.17): the service holds a power assertion
//! while any agent is working or has subagents at work and the `agents.keep_awake` setting is
//! on, and releases it when none is. Only macOS has one ([`crate::macos::power`]); elsewhere
//! the service holds nothing.

use crate::states::Agent;

/// Set (to anything) in a service's environment, the service never takes the power assertion:
/// the tests' services, which must not keep the machine running them awake.
pub const OFF: &str = "HIVE_NO_POWER_ASSERTION";

/// A way to keep the computer from sleeping while idle. Each `hold` is followed by a
/// `release` before the next `hold`.
pub trait Power: Send {
    /// Takes the assertion. A refusal is the implementation's to report.
    fn hold(&mut self);
    /// Gives the assertion back, if it was taken.
    fn release(&mut self);
}

/// The service's power assertion: taken and released only when whether it is wanted changes.
pub struct Awake {
    /// `None`: the system has none (Linux, Windows) or it is turned off (tests).
    power: Option<Box<dyn Power>>,
    wanted: bool,
}

impl Awake {
    pub fn new(power: Option<Box<dyn Power>>) -> Self {
        Self {
            power,
            wanted: false,
        }
    }

    /// Holds the assertion while `wanted`.
    pub fn set(&mut self, wanted: bool) {
        if wanted == self.wanted {
            return;
        }
        self.wanted = wanted;
        let Some(power) = &mut self.power else { return };
        if wanted {
            power.hold();
        } else {
            power.release();
        }
    }
}

/// Whether the computer is kept awake: the setting is `on` and some agent works.
pub fn wanted<'a>(on: bool, mut agents: impl Iterator<Item = &'a Agent>) -> bool {
    on && agents.any(Agent::works)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    /// Records each call: `true` for a hold, `false` for a release.
    #[derive(Clone, Default)]
    pub struct Calls(pub Arc<Mutex<Vec<bool>>>);

    impl Calls {
        pub fn power(&self) -> Option<Box<dyn Power>> {
            Some(Box::new(self.clone()))
        }

        pub fn taken(&self) -> Vec<bool> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Power for Calls {
        fn hold(&mut self) {
            self.0.lock().unwrap().push(true);
        }

        fn release(&mut self) {
            self.0.lock().unwrap().push(false);
        }
    }

    #[test]
    fn the_assertion_is_taken_and_released_once_per_change() {
        let calls = Calls::default();
        let mut awake = Awake::new(calls.power());
        awake.set(false);
        assert!(calls.taken().is_empty());
        awake.set(true);
        awake.set(true);
        assert_eq!(calls.taken(), vec![true]);
        awake.set(false);
        awake.set(false);
        awake.set(true);
        assert_eq!(calls.taken(), vec![true, false, true]);
    }

    #[test]
    fn without_a_power_assertion_nothing_is_held() {
        let mut awake = Awake::new(None);
        awake.set(true);
        assert!(awake.wanted);
        awake.set(false);
        assert!(!awake.wanted);
    }

    #[test]
    fn it_is_wanted_while_the_setting_is_on_and_an_agent_works() {
        let now = Instant::now();
        let idle = Agent::new(1, now, 0);
        let working = Agent::working(2, now);
        assert!(wanted(true, [&idle, &working].into_iter()));
        assert!(!wanted(false, [&idle, &working].into_iter()));
        assert!(!wanted(true, [&idle].into_iter()));
        assert!(!wanted(true, std::iter::empty()));
    }
}
