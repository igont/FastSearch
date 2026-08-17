use std::time::Duration;

const RATE_SMOOTHING_ALPHA: f64 = 0.25;

#[derive(Clone, Debug, Default)]
pub(super) struct ProgressEstimator {
    last_completed: u64,
    last_elapsed: Duration,
    smoothed_rate: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ProgressForecast {
    rate_per_second: f64,
    remaining: Duration,
}

impl ProgressEstimator {
    pub(super) fn observe(
        &mut self,
        completed: u64,
        total: u64,
        elapsed: Duration,
    ) -> Option<ProgressForecast> {
        if completed < self.last_completed || elapsed < self.last_elapsed {
            *self = Self::default();
        }
        let completed_delta = completed.saturating_sub(self.last_completed);
        let elapsed_delta = elapsed.saturating_sub(self.last_elapsed);
        if completed_delta > 0 && !elapsed_delta.is_zero() {
            let sample_rate = completed_delta as f64 / elapsed_delta.as_secs_f64();
            self.smoothed_rate = Some(match self.smoothed_rate {
                Some(previous) => {
                    previous * (1.0 - RATE_SMOOTHING_ALPHA) + sample_rate * RATE_SMOOTHING_ALPHA
                }
                None => sample_rate,
            });
        }
        self.last_completed = completed;
        self.last_elapsed = elapsed;

        let rate_per_second = self.smoothed_rate.filter(|rate| *rate > 0.0)?;
        let remaining_records = total.saturating_sub(completed);
        Some(ProgressForecast {
            rate_per_second,
            remaining: Duration::from_secs_f64(remaining_records as f64 / rate_per_second),
        })
    }
}

impl ProgressForecast {
    pub(super) fn detail(self) -> String {
        let rate = if self.rate_per_second >= 10.0 {
            format!("{:.0} зап./с", self.rate_per_second)
        } else {
            format!("{:.1} зап./с", self.rate_per_second)
        };
        format!(
            "Текущая скорость: {rate} · осталось около {}.",
            human_duration(self.remaining)
        )
    }

    pub(super) fn download_detail(self) -> String {
        format!(
            "Текущая скорость: {}/с · осталось около {}.",
            human_bytes(self.rate_per_second),
            human_duration(self.remaining)
        )
    }
}

pub(super) fn human_duration(duration: Duration) -> String {
    let seconds = duration.as_secs_f64().ceil().max(1.0) as u64;
    if seconds < 60 {
        return format!("{seconds} сек");
    }
    let minutes = seconds.div_ceil(60);
    if minutes < 60 {
        return format!("{minutes} мин");
    }
    let hours = minutes / 60;
    let remaining_minutes = minutes % 60;
    if remaining_minutes == 0 {
        format!("{hours} ч")
    } else {
        format!("{hours} ч {remaining_minutes} мин")
    }
}

fn human_bytes(bytes_per_second: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    if bytes_per_second >= MIB {
        format!("{:.1} МиБ", bytes_per_second / MIB)
    } else if bytes_per_second >= KIB {
        format!("{:.1} КиБ", bytes_per_second / KIB)
    } else {
        format!("{bytes_per_second:.0} Б")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forecast_uses_measured_rate_and_smoothly_corrects_it() {
        let mut estimator = ProgressEstimator::default();
        assert!(estimator.observe(0, 100, Duration::from_secs(0)).is_none());

        let first = estimator
            .observe(8, 100, Duration::from_secs(4))
            .expect("first completed chunk establishes throughput");
        let corrected = estimator
            .observe(16, 100, Duration::from_secs(6))
            .expect("next chunk corrects the moving estimate");

        assert!((first.rate_per_second - 2.0).abs() < f64::EPSILON);
        assert!((corrected.rate_per_second - 2.5).abs() < f64::EPSILON);
        assert!(corrected.remaining < first.remaining);
        assert_eq!(
            corrected.detail(),
            "Текущая скорость: 2.5 зап./с · осталось около 34 сек."
        );
    }

    #[test]
    fn duration_is_compact_for_seconds_minutes_and_hours() {
        assert_eq!(human_duration(Duration::from_secs(9)), "9 сек");
        assert_eq!(human_duration(Duration::from_secs(61)), "2 мин");
        assert_eq!(human_duration(Duration::from_secs(3_661)), "1 ч 2 мин");
    }

    #[test]
    fn download_forecast_uses_binary_transfer_units() {
        let forecast = ProgressForecast {
            rate_per_second: 2.5 * 1024.0 * 1024.0,
            remaining: Duration::from_secs(61),
        };
        assert_eq!(
            forecast.download_detail(),
            "Текущая скорость: 2.5 МиБ/с · осталось около 2 мин."
        );
    }
}
