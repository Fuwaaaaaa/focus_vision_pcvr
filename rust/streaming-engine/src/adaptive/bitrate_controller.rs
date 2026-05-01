use std::time::{Duration, Instant};
use crate::adaptive::bandwidth_estimator::BandwidthEstimator;
use super::burst_detector::{BurstDetector, LossPattern};
use super::gcc_estimator::GccEstimator;

/// Reduced-form network signals fed into the arbitration step. Decoupled from
/// the live BandwidthEstimator/GccEstimator/BurstDetector so the priority
/// logic can be unit-tested with synthetic scalars.
#[derive(Debug, Clone, Copy)]
pub struct ArbitrationInputs {
    pub current_bps: u64,
    pub loss_rate: f64,
    pub delay_gradient_ms: f64,
    pub burst: LossPattern,
    /// Whether the upward-adjustment cooldown has elapsed since the last
    /// bitrate change (the controller's hysteresis gate).
    pub hysteresis_elapsed: bool,
    pub target_loss_rate: f64,
    pub min_bps: u64,
    pub hard_max_bps: u64,
    pub thermal_ceiling_bps: Option<u64>,
}

/// Which signal forced the arbitrated decision. Useful for log/telemetry
/// so operators can answer "why did the bitrate just drop?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArbDominant {
    /// No change requested — current bitrate is the target.
    None,
    /// Sustained-loss aggressive reduction (-20%).
    Sustained,
    /// Loss-based reduction picked over delay-based.
    Loss,
    /// Delay/GCC-based reduction (gradient overuse).
    Delay,
    /// Thermal ceiling clamped the network-derived target.
    Thermal,
    /// Hysteresis-gated upward step (+5%).
    Increase,
    /// Floor (min_bps) protected against an over-aggressive cap.
    Floor,
}

#[derive(Debug, Clone, Copy)]
pub struct ArbitrationOutcome {
    pub target_bps: u64,
    pub dominant: ArbDominant,
}

/// Pure arbitration: given current bitrate plus reduced network signals,
/// decide the new target and the dominant signal. The network multiplier
/// chain (sustained > delay+loss > increase) is computed first; the result
/// is then clamped against the floor/hard_max/thermal stack with floor
/// taking absolute priority.
pub fn arbitrate(inputs: &ArbitrationInputs) -> ArbitrationOutcome {
    let (mut multiplier, mut network_dom) = (1.0f64, ArbDominant::None);

    // Burst pattern: skip changes entirely so FEC absorbs Wi-Fi interference.
    if inputs.burst == LossPattern::Burst {
        let target = clamp_with_thermal(
            inputs.current_bps,
            inputs.min_bps,
            inputs.hard_max_bps,
            inputs.thermal_ceiling_bps,
        );
        let dom = decide_clamp_dominant(inputs.current_bps, target,
            inputs.thermal_ceiling_bps, ArbDominant::None);
        return ArbitrationOutcome { target_bps: target, dominant: dom };
    }

    if inputs.burst == LossPattern::Sustained {
        multiplier = 0.80;
        network_dom = ArbDominant::Sustained;
    } else {
        if inputs.delay_gradient_ms > 2.0 {
            let m = 0.90;
            if m < multiplier {
                multiplier = m;
                network_dom = ArbDominant::Delay;
            }
        }
        if inputs.loss_rate > 0.05 {
            let m = 0.80;
            if m < multiplier {
                multiplier = m;
                network_dom = ArbDominant::Loss;
            }
        } else if inputs.loss_rate > inputs.target_loss_rate {
            let m = 0.95;
            if m < multiplier {
                multiplier = m;
                network_dom = ArbDominant::Loss;
            }
        }
    }

    if multiplier >= 1.0 && inputs.hysteresis_elapsed && inputs.loss_rate < 0.01 {
        multiplier = 1.05;
        network_dom = ArbDominant::Increase;
    }

    let candidate = (inputs.current_bps as f64 * multiplier) as u64;
    let target = clamp_with_thermal(
        candidate,
        inputs.min_bps,
        inputs.hard_max_bps,
        inputs.thermal_ceiling_bps,
    );
    let dom = decide_clamp_dominant(candidate, target,
        inputs.thermal_ceiling_bps, network_dom);

    ArbitrationOutcome { target_bps: target, dominant: dom }
}

/// Apply the floor / hard_max / thermal_ceiling stack. Floor wins absolutely:
/// even a thermal ceiling below `min_bps` cannot crush the encoder below its
/// minimum operating point.
fn clamp_with_thermal(candidate: u64, min: u64, hard_max: u64, thermal: Option<u64>) -> u64 {
    let effective_max = match thermal {
        Some(t) => hard_max.min(t),
        None => hard_max,
    };
    candidate.min(effective_max).max(min)
}

/// Decide which signal "won" after clamping. If the clamp moved the candidate
/// downward, the cap that bit is the dominant signal; floor wins if it raised
/// the candidate up.
fn decide_clamp_dominant(
    candidate: u64,
    final_target: u64,
    thermal: Option<u64>,
    network_dom: ArbDominant,
) -> ArbDominant {
    if final_target > candidate {
        return ArbDominant::Floor;
    }
    if let Some(t) = thermal {
        if final_target == t && t < candidate {
            return ArbDominant::Thermal;
        }
    }
    network_dom
}

/// Adaptive bitrate controller.
/// Adjusts encoding bitrate based on network quality estimates.
pub struct BitrateController {
    current_bitrate_bps: u64,
    min_bitrate_bps: u64,
    max_bitrate_bps: u64,
    /// Optional ceiling imposed by the thermal governor. When `Some`, every
    /// computed bitrate is additionally clamped to `min(max_bitrate_bps, this)`
    /// while the floor still wins if the ceiling drops below `min_bitrate_bps`.
    thermal_ceiling_bps: Option<u64>,
    target_loss_rate: f64,
    last_adjustment: Instant,
    /// Minimum interval between upward adjustments (hysteresis)
    hysteresis_duration: Duration,
    /// Reason the most recent `adjust()` call resolved the way it did. Useful
    /// for log lines and the `/status` JSON. `ArbDominant::None` until the
    /// first call that produced a decision (including no-change cases once
    /// data is available).
    last_decision: ArbDominant,
}

impl BitrateController {
    pub fn new(initial_bitrate_mbps: u32) -> Self {
        Self {
            current_bitrate_bps: initial_bitrate_mbps as u64 * 1_000_000,
            min_bitrate_bps: 10_000_000,   // 10 Mbps floor
            max_bitrate_bps: 200_000_000,  // 200 Mbps ceiling
            thermal_ceiling_bps: None,
            target_loss_rate: 0.02,        // 2%
            last_adjustment: Instant::now(),
            hysteresis_duration: Duration::from_secs(10),
            last_decision: ArbDominant::None,
        }
    }

    /// Constructor with custom hysteresis duration (for testing).
    #[cfg(test)]
    pub(crate) fn new_with_hysteresis(initial_bitrate_mbps: u32, hysteresis: Duration) -> Self {
        Self {
            hysteresis_duration: hysteresis,
            ..Self::new(initial_bitrate_mbps)
        }
    }

    /// Evaluate network conditions and adjust bitrate.
    /// Call this periodically (every ~1 second).
    /// Returns true if bitrate was changed.
    pub fn adjust(&mut self, estimator: &BandwidthEstimator, gcc: &GccEstimator, burst: &BurstDetector) -> bool {
        if !estimator.has_data() {
            return false;
        }

        let inputs = ArbitrationInputs {
            current_bps: self.current_bitrate_bps,
            loss_rate: estimator.loss_rate(),
            delay_gradient_ms: gcc.delay_gradient_ms(),
            burst: burst.pattern(),
            hysteresis_elapsed: self.last_adjustment.elapsed() > self.hysteresis_duration,
            target_loss_rate: self.target_loss_rate,
            min_bps: self.min_bitrate_bps,
            hard_max_bps: self.max_bitrate_bps,
            thermal_ceiling_bps: self.thermal_ceiling_bps,
        };
        let outcome = arbitrate(&inputs);
        self.last_decision = outcome.dominant;

        if outcome.target_bps != self.current_bitrate_bps {
            log::info!(
                "Bitrate {} → {} Mbps (signal: {:?})",
                self.current_bitrate_bps / 1_000_000,
                outcome.target_bps / 1_000_000,
                outcome.dominant,
            );
            self.current_bitrate_bps = outcome.target_bps;
            self.last_adjustment = Instant::now();
            true
        } else {
            false
        }
    }

    /// Which signal forced the most recent decision (or `None` before the
    /// first `adjust()` call). Read by the status writer / log layer.
    pub fn last_decision(&self) -> ArbDominant {
        self.last_decision
    }

    /// Apply the floor / ceiling / thermal-ceiling stack to a candidate bitrate.
    /// Floor always wins — even a thermal ceiling below `min_bitrate_bps` cannot
    /// reduce the encoder below its minimum operating point.
    fn clamp_to_bounds(&self, candidate_bps: u64) -> u64 {
        let effective_max = match self.thermal_ceiling_bps {
            Some(thermal) => self.max_bitrate_bps.min(thermal),
            None => self.max_bitrate_bps,
        };
        candidate_bps.min(effective_max).max(self.min_bitrate_bps)
    }

    /// Set the thermal ceiling (in bps). Immediately clamps the live bitrate
    /// if it sits above the new ceiling so the cap takes effect on the next
    /// frame, not the next adjustment cycle.
    pub fn set_thermal_ceiling_bps(&mut self, ceiling_bps: u64) {
        self.thermal_ceiling_bps = Some(ceiling_bps);
        let clamped = self.clamp_to_bounds(self.current_bitrate_bps);
        if clamped != self.current_bitrate_bps {
            log::info!(
                "Thermal ceiling applied: {} → {} Mbps",
                self.current_bitrate_bps / 1_000_000,
                clamped / 1_000_000,
            );
            self.current_bitrate_bps = clamped;
        }
    }

    /// Lift the thermal ceiling — the live bitrate stays where it is, but
    /// future adjustments may grow back up to `max_bitrate_bps`.
    pub fn clear_thermal_ceiling(&mut self) {
        self.thermal_ceiling_bps = None;
    }

    pub fn thermal_ceiling_bps(&self) -> Option<u64> {
        self.thermal_ceiling_bps
    }

    pub fn current_bitrate_bps(&self) -> u64 { self.current_bitrate_bps }
    pub fn current_bitrate_mbps(&self) -> u32 { (self.current_bitrate_bps / 1_000_000) as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_bitrate() {
        let ctrl = BitrateController::new(80);
        assert_eq!(ctrl.current_bitrate_mbps(), 80);
    }

    #[test]
    fn test_high_loss_reduces_bitrate() {
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(50, 50, 10.0); // 50% loss
        ctrl.adjust(&est, &gcc, &burst);
        assert!(ctrl.current_bitrate_mbps() < 100);
    }

    #[test]
    fn test_no_loss_no_immediate_increase() {
        let mut ctrl = BitrateController::new(80);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(100, 0, 5.0); // 0% loss
        let changed = ctrl.adjust(&est, &gcc, &burst);
        // Should not increase yet (hysteresis)
        assert!(!changed);
        assert_eq!(ctrl.current_bitrate_mbps(), 80);
    }

    #[test]
    fn test_floor_enforced() {
        let mut ctrl = BitrateController::new(11);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(10, 90, 100.0); // 90% loss - extreme
        ctrl.adjust(&est, &gcc, &burst); // → 11 * 0.8 = 8.8 → clamped to 10
        assert_eq!(ctrl.current_bitrate_mbps(), 10);
    }

    #[test]
    fn test_moderate_loss_gentle_reduction() {
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(97, 3, 8.0); // 3% loss
        ctrl.adjust(&est, &gcc, &burst);
        assert_eq!(ctrl.current_bitrate_mbps(), 95); // -5%
    }

    #[test]
    fn test_adjust_overuse_without_loss() {
        use fvp_common::protocol::TransportFeedbackEntry;
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let mut gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(100, 0, 5.0); // 0% loss

        // Simulate congestion: increasing inter-arrival deltas
        let entries = vec![
            TransportFeedbackEntry { sequence: 0, recv_delta_us: 10_000 },
            TransportFeedbackEntry { sequence: 1, recv_delta_us: 13_000 },
            TransportFeedbackEntry { sequence: 2, recv_delta_us: 17_000 },
            TransportFeedbackEntry { sequence: 3, recv_delta_us: 22_000 },
        ];
        gcc.process_feedback(&entries);
        assert!(gcc.delay_gradient_ms() > 2.0, "gradient should be >2.0, got {}", gcc.delay_gradient_ms());

        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(changed, "Bitrate should have decreased");
        assert!(ctrl.current_bitrate_mbps() < 100, "Expected reduction, got {}", ctrl.current_bitrate_mbps());
    }

    #[test]
    fn test_adjust_delay_and_loss_combined() {
        use fvp_common::protocol::TransportFeedbackEntry;
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let mut gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(90, 10, 10.0); // 10% loss (high)

        // Also simulate delay overuse
        let entries = vec![
            TransportFeedbackEntry { sequence: 0, recv_delta_us: 10_000 },
            TransportFeedbackEntry { sequence: 1, recv_delta_us: 15_000 },
            TransportFeedbackEntry { sequence: 2, recv_delta_us: 22_000 },
        ];
        gcc.process_feedback(&entries);

        ctrl.adjust(&est, &gcc, &burst);
        // Max-of-reductions: loss -20% dominates delay -10%, so 100 * 0.80 = 80 Mbps
        assert_eq!(ctrl.current_bitrate_mbps(), 80, "Expected max reduction (0.80), got {}", ctrl.current_bitrate_mbps());
    }

    #[test]
    fn test_underuse_increases_bitrate() {
        use fvp_common::protocol::TransportFeedbackEntry;
        // Use short hysteresis so we don't need to sleep 10 seconds
        let mut ctrl = BitrateController::new_with_hysteresis(100, Duration::from_millis(10));
        let mut est = BandwidthEstimator::new();
        let mut gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(100, 0, 5.0); // 0% loss

        // Simulate delay recovery: decreasing inter-arrival deltas (gradient < -1.0)
        let entries = vec![
            TransportFeedbackEntry { sequence: 0, recv_delta_us: 20_000 },
            TransportFeedbackEntry { sequence: 1, recv_delta_us: 17_000 },
            TransportFeedbackEntry { sequence: 2, recv_delta_us: 13_000 },
            TransportFeedbackEntry { sequence: 3, recv_delta_us: 8_000 },
        ];
        gcc.process_feedback(&entries);
        assert!(gcc.delay_gradient_ms() < -1.0, "gradient should be < -1.0, got {}", gcc.delay_gradient_ms());

        // Wait for hysteresis to elapse
        std::thread::sleep(Duration::from_millis(20));

        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(changed, "Bitrate should have increased");
        assert_eq!(ctrl.current_bitrate_mbps(), 105, "Expected +5% increase, got {}", ctrl.current_bitrate_mbps());
    }

    #[test]
    fn test_ceiling_enforced() {
        let mut ctrl = BitrateController::new_with_hysteresis(195, Duration::from_millis(10));
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(100, 0, 5.0); // 0% loss, no congestion

        // Wait for hysteresis to elapse
        std::thread::sleep(Duration::from_millis(20));

        ctrl.adjust(&est, &gcc, &burst);
        // 195 * 1.05 = 204.75 → clamped to 200 Mbps ceiling
        assert_eq!(ctrl.current_bitrate_mbps(), 200, "Expected ceiling at 200 Mbps, got {}", ctrl.current_bitrate_mbps());
    }

    #[test]
    fn test_no_change_without_data() {
        let mut ctrl = BitrateController::new(100);
        let est = BandwidthEstimator::new(); // no data fed
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(!changed, "Should not change without data");
        assert_eq!(ctrl.current_bitrate_mbps(), 100);
    }

    #[test]
    fn test_burst_suppresses_reduction() {
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let mut burst = BurstDetector::new();
        est.update(50, 50, 10.0); // 50% loss — very high
        burst.record(0.50); // Triggers Burst pattern
        assert_eq!(burst.pattern(), LossPattern::Burst);

        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(!changed, "Burst should suppress bitrate reduction");
        assert_eq!(ctrl.current_bitrate_mbps(), 100, "Bitrate should remain unchanged during burst");
    }

    // -- Pure arbitrate() function tests (no estimator/gcc/burst plumbing) --
    //
    // Inputs are network signals already reduced to scalars; outputs the
    // arbitrated target bitrate plus the dominant signal name. This keeps
    // the priority logic verifiable in isolation from the noise of the
    // bandwidth/GCC/burst detector update paths.

    #[test]
    fn test_arbitrate_no_signal_holds_current() {
        let inputs = ArbitrationInputs {
            current_bps: 80_000_000,
            loss_rate: 0.0,
            delay_gradient_ms: 0.0,
            burst: LossPattern::None,
            hysteresis_elapsed: false,
            target_loss_rate: 0.02,
            min_bps: 10_000_000,
            hard_max_bps: 200_000_000,
            thermal_ceiling_bps: None,
        };
        let out = arbitrate(&inputs);
        assert_eq!(out.target_bps, 80_000_000);
        assert_eq!(out.dominant, ArbDominant::None);
    }

    #[test]
    fn test_arbitrate_thermal_dominates_when_below_current() {
        let inputs = ArbitrationInputs {
            current_bps: 100_000_000,
            loss_rate: 0.0,
            delay_gradient_ms: 0.0,
            burst: LossPattern::None,
            hysteresis_elapsed: false,
            target_loss_rate: 0.02,
            min_bps: 10_000_000,
            hard_max_bps: 200_000_000,
            thermal_ceiling_bps: Some(60_000_000),
        };
        let out = arbitrate(&inputs);
        assert_eq!(out.target_bps, 60_000_000, "thermal cap should clamp from 100 → 60 Mbps");
        assert_eq!(out.dominant, ArbDominant::Thermal);
    }

    #[test]
    fn test_arbitrate_loss_dominates_thermal_when_more_aggressive() {
        // High loss → 80% of current = 80 Mbps. Thermal ceiling at 90 Mbps
        // is more permissive. Loss wins.
        let inputs = ArbitrationInputs {
            current_bps: 100_000_000,
            loss_rate: 0.10,
            delay_gradient_ms: 0.0,
            burst: LossPattern::None,
            hysteresis_elapsed: false,
            target_loss_rate: 0.02,
            min_bps: 10_000_000,
            hard_max_bps: 200_000_000,
            thermal_ceiling_bps: Some(90_000_000),
        };
        let out = arbitrate(&inputs);
        assert_eq!(out.target_bps, 80_000_000, "loss reduction (80) beats thermal (90)");
        assert_eq!(out.dominant, ArbDominant::Loss);
    }

    #[test]
    fn test_arbitrate_thermal_dominates_loss_when_more_aggressive() {
        // Moderate loss → 95% of current = 95 Mbps. Thermal ceiling at 60 Mbps
        // is much more aggressive. Thermal wins.
        let inputs = ArbitrationInputs {
            current_bps: 100_000_000,
            loss_rate: 0.03, // moderate (above target 0.02)
            delay_gradient_ms: 0.0,
            burst: LossPattern::None,
            hysteresis_elapsed: false,
            target_loss_rate: 0.02,
            min_bps: 10_000_000,
            hard_max_bps: 200_000_000,
            thermal_ceiling_bps: Some(60_000_000),
        };
        let out = arbitrate(&inputs);
        assert_eq!(out.target_bps, 60_000_000, "thermal (60) beats loss reduction (95)");
        assert_eq!(out.dominant, ArbDominant::Thermal);
    }

    #[test]
    fn test_arbitrate_floor_respected_under_pathological_inputs() {
        // Sustained burst (× 0.80) + thermal cap below floor — neither may
        // crush the encoder below its minimum operating point.
        let inputs = ArbitrationInputs {
            current_bps: 12_000_000,
            loss_rate: 0.50,
            delay_gradient_ms: 100.0,
            burst: LossPattern::Sustained,
            hysteresis_elapsed: false,
            target_loss_rate: 0.02,
            min_bps: 10_000_000,
            hard_max_bps: 200_000_000,
            thermal_ceiling_bps: Some(2_000_000), // below floor
        };
        let out = arbitrate(&inputs);
        assert_eq!(out.target_bps, 10_000_000, "floor must hold against thermal+sustained");
    }

    #[test]
    fn test_arbitrate_thermal_blocks_increase_attempt() {
        // All-clear signals + hysteresis elapsed would normally yield +5%,
        // but a thermal cap below the +5% target must clamp the rise.
        let inputs = ArbitrationInputs {
            current_bps: 100_000_000,
            loss_rate: 0.0,
            delay_gradient_ms: -2.0, // recovery
            burst: LossPattern::None,
            hysteresis_elapsed: true,
            target_loss_rate: 0.02,
            min_bps: 10_000_000,
            hard_max_bps: 200_000_000,
            thermal_ceiling_bps: Some(100_000_000), // cap = current
        };
        let out = arbitrate(&inputs);
        assert_eq!(out.target_bps, 100_000_000, "thermal cap blocks +5% rise from 100 → 105");
        assert_eq!(out.dominant, ArbDominant::Thermal);
    }

    #[test]
    fn test_thermal_ceiling_clamps_current_bitrate_immediately() {
        let mut ctrl = BitrateController::new(150);
        ctrl.set_thermal_ceiling_bps(70_000_000);
        assert_eq!(ctrl.current_bitrate_mbps(), 70,
            "thermal ceiling must clamp the live bitrate, not just future adjustments");
    }

    #[test]
    fn test_thermal_ceiling_no_effect_when_above_current() {
        let mut ctrl = BitrateController::new(80);
        ctrl.set_thermal_ceiling_bps(150_000_000);
        assert_eq!(ctrl.current_bitrate_mbps(), 80,
            "ceiling above current bitrate is a no-op");
    }

    #[test]
    fn test_thermal_ceiling_clamps_upward_adjust() {
        let mut ctrl = BitrateController::new_with_hysteresis(80, Duration::from_millis(10));
        ctrl.set_thermal_ceiling_bps(85_000_000);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let burst = BurstDetector::new();
        est.update(100, 0, 5.0);
        std::thread::sleep(Duration::from_millis(20));
        ctrl.adjust(&est, &gcc, &burst);
        // 80 * 1.05 = 84 (under ceiling) — but next tick would push to 88 which would clamp to 85.
        // Verify the next bump respects the ceiling.
        std::thread::sleep(Duration::from_millis(20));
        ctrl.adjust(&est, &gcc, &burst);
        assert!(ctrl.current_bitrate_mbps() <= 85,
            "ceiling must hold across consecutive +5% adjustments, got {}",
            ctrl.current_bitrate_mbps());
    }

    #[test]
    fn test_thermal_ceiling_lifted_allows_unlimited_growth() {
        let mut ctrl = BitrateController::new(150);
        ctrl.set_thermal_ceiling_bps(70_000_000);
        assert_eq!(ctrl.current_bitrate_mbps(), 70);
        // Lift ceiling — current bitrate stays at 70 (no spontaneous jump),
        // but the cap is removed so future adjust() can grow back.
        ctrl.clear_thermal_ceiling();
        assert_eq!(ctrl.current_bitrate_mbps(), 70);
        assert_eq!(ctrl.thermal_ceiling_bps(), None);
    }

    #[test]
    fn test_thermal_ceiling_respects_floor() {
        // Ceiling below the 10 Mbps floor — floor wins.
        let mut ctrl = BitrateController::new(100);
        ctrl.set_thermal_ceiling_bps(5_000_000);
        assert_eq!(ctrl.current_bitrate_mbps(), 10,
            "thermal ceiling below floor must not crush below min");
    }

    #[test]
    fn test_thermal_ceiling_returns_stored_value() {
        let mut ctrl = BitrateController::new(80);
        assert_eq!(ctrl.thermal_ceiling_bps(), None);
        ctrl.set_thermal_ceiling_bps(60_000_000);
        assert_eq!(ctrl.thermal_ceiling_bps(), Some(60_000_000));
    }

    #[test]
    fn test_sustained_triggers_aggressive_reduction() {
        let mut ctrl = BitrateController::new(100);
        let mut est = BandwidthEstimator::new();
        let gcc = GccEstimator::new(80_000_000);
        let mut burst = BurstDetector::new();
        est.update(90, 10, 10.0); // 10% loss
        burst.record(0.10); // Start burst
        // Wait beyond sustained threshold
        std::thread::sleep(Duration::from_millis(600));
        burst.record(0.10); // Now sustained
        assert_eq!(burst.pattern(), LossPattern::Sustained);

        let changed = ctrl.adjust(&est, &gcc, &burst);
        assert!(changed, "Sustained loss should trigger bitrate change");
        assert_eq!(ctrl.current_bitrate_mbps(), 80, "Expected aggressive -20% reduction, got {}", ctrl.current_bitrate_mbps());
    }
}
