use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct UpdateAutoCheckPolicy {
    interval: Duration,
    startup_check_done: bool,
    last_check_started_at: Option<Instant>,
}

impl UpdateAutoCheckPolicy {
    #[must_use]
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            startup_check_done: false,
            last_check_started_at: None,
        }
    }

    #[must_use]
    pub fn should_start_check(&mut self, enabled: bool, now: Instant) -> bool {
        if !enabled {
            return false;
        }
        if !self.startup_check_done {
            self.startup_check_done = true;
            self.last_check_started_at = Some(now);
            return true;
        }
        if self
            .last_check_started_at
            .is_some_and(|last| now.duration_since(last) >= self.interval)
        {
            self.last_check_started_at = Some(now);
            return true;
        }
        false
    }

    pub fn note_manual_check_started(&mut self, now: Instant) {
        self.last_check_started_at = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_check_fires_once_when_enabled() {
        let start = Instant::now();
        let mut policy = UpdateAutoCheckPolicy::new(Duration::from_secs(60));

        assert!(policy.should_start_check(true, start));
        assert!(!policy.should_start_check(true, start + Duration::from_secs(59)));
    }

    #[test]
    fn interval_check_fires_after_cadence() {
        let start = Instant::now();
        let mut policy = UpdateAutoCheckPolicy::new(Duration::from_secs(60));

        assert!(policy.should_start_check(true, start));
        assert!(!policy.should_start_check(true, start + Duration::from_secs(59)));
        assert!(policy.should_start_check(true, start + Duration::from_secs(60)));
    }

    #[test]
    fn disabled_auto_check_never_fires() {
        let start = Instant::now();
        let mut policy = UpdateAutoCheckPolicy::new(Duration::from_secs(60));

        assert!(!policy.should_start_check(false, start));
    }

    #[test]
    fn manual_check_resets_interval() {
        let start = Instant::now();
        let mut policy = UpdateAutoCheckPolicy::new(Duration::from_secs(60));

        assert!(policy.should_start_check(true, start));
        policy.note_manual_check_started(start + Duration::from_secs(50));
        assert!(!policy.should_start_check(true, start + Duration::from_secs(100)));
        assert!(policy.should_start_check(true, start + Duration::from_secs(110)));
    }
}
