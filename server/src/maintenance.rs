//! Planificateur de maintenance — pur, testé en isolation. Diffuse des messages staff à
//! H-60/H-15/H-5 avant une maintenance programmée, puis signale le moment du drain à H.

use std::time::SystemTime;

#[derive(Debug, Clone, Copy)]
pub struct MaintenanceSchedule {
    /// Instant Unix (secondes) auquel la maintenance doit démarrer.
    pub scheduled_at_unix: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WarningStage {
    H60,
    H15,
    H5,
}

impl MaintenanceSchedule {
    /// Renvoie le palier d'avertissement à diffuser MAINTENANT, si `now` vient de franchir un
    /// des seuils H-60/H-15/H-5 — `already_sent` évite de rediffuser le même palier deux fois.
    pub fn warning_stage_due(
        &self,
        now: SystemTime,
        already_sent: &[WarningStage],
    ) -> Option<WarningStage> {
        let now_unix = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now_unix >= self.scheduled_at_unix {
            return None; // maintenance déjà due, plus un avertissement
        }
        let remaining = self.scheduled_at_unix - now_unix;
        let stage = if remaining <= 5 * 60 {
            WarningStage::H5
        } else if remaining <= 15 * 60 {
            WarningStage::H15
        } else if remaining <= 60 * 60 {
            WarningStage::H60
        } else {
            return None;
        };
        if already_sent.contains(&stage) {
            None
        } else {
            Some(stage)
        }
    }

    /// Le drain doit-il démarrer maintenant ?
    pub fn should_drain_now(&self, now: SystemTime) -> bool {
        let now_unix = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now_unix >= self.scheduled_at_unix
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn unix_time(secs: u64) -> SystemTime {
        std::time::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn warning_stage_due_returns_h60_at_exactly_60_minutes_remaining() {
        let sched = MaintenanceSchedule {
            scheduled_at_unix: 10_000,
        };
        let now = unix_time(10_000 - 60 * 60);
        assert_eq!(sched.warning_stage_due(now, &[]), Some(WarningStage::H60));
    }

    #[test]
    fn warning_stage_due_returns_none_more_than_60_minutes_out() {
        let sched = MaintenanceSchedule {
            scheduled_at_unix: 10_000,
        };
        let now = unix_time(10_000 - 60 * 60 - 1);
        assert_eq!(sched.warning_stage_due(now, &[]), None);
    }

    #[test]
    fn warning_stage_due_does_not_resend_an_already_sent_stage() {
        let sched = MaintenanceSchedule {
            scheduled_at_unix: 10_000,
        };
        let now = unix_time(10_000 - 60 * 60);
        assert_eq!(
            sched.warning_stage_due(now, &[WarningStage::H60]),
            None,
            "H60 déjà diffusé ne doit pas se répéter"
        );
    }

    #[test]
    fn warning_stage_due_returns_h15_when_within_that_window() {
        let sched = MaintenanceSchedule {
            scheduled_at_unix: 10_000,
        };
        let now = unix_time(10_000 - 15 * 60);
        assert_eq!(sched.warning_stage_due(now, &[]), Some(WarningStage::H15));
    }

    #[test]
    fn warning_stage_due_returns_h5_when_within_that_window() {
        let sched = MaintenanceSchedule {
            scheduled_at_unix: 10_000,
        };
        let now = unix_time(10_000 - 5 * 60);
        assert_eq!(sched.warning_stage_due(now, &[]), Some(WarningStage::H5));
    }

    #[test]
    fn warning_stage_due_returns_none_once_maintenance_is_already_due() {
        let sched = MaintenanceSchedule {
            scheduled_at_unix: 10_000,
        };
        let now = unix_time(10_000);
        assert_eq!(sched.warning_stage_due(now, &[]), None);
    }

    #[test]
    fn should_drain_now_is_false_before_the_scheduled_time() {
        let sched = MaintenanceSchedule {
            scheduled_at_unix: 10_000,
        };
        assert!(!sched.should_drain_now(unix_time(9_999)));
    }

    #[test]
    fn should_drain_now_is_true_at_or_after_the_scheduled_time() {
        let sched = MaintenanceSchedule {
            scheduled_at_unix: 10_000,
        };
        assert!(sched.should_drain_now(unix_time(10_000)));
        assert!(sched.should_drain_now(unix_time(10_001)));
    }
}
