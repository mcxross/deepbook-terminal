use rust_decimal::Decimal;
use std::cmp::Ordering;

pub trait Sign {
    fn positive(&self) -> bool;
    fn negative(&self) -> bool;
    fn zero(&self) -> bool;
    fn sign(&self) -> Ordering;
}

impl Sign for str {
    fn positive(&self) -> bool {
        !(self.negative() || self.zero())
    }

    fn negative(&self) -> bool {
        self.starts_with('-')
    }

    fn zero(&self) -> bool {
        self.chars().all(|c| matches!(c, '0' | '.' | '+' | '-'))
    }

    fn sign(&self) -> Ordering {
        if self.negative() {
            Ordering::Less
        } else if self.zero() {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

impl Sign for rust_decimal::Decimal {
    fn positive(&self) -> bool {
        self.is_sign_positive() && !self.is_zero()
    }

    fn negative(&self) -> bool {
        self.is_sign_negative()
    }

    fn zero(&self) -> bool {
        self.is_zero()
    }

    fn sign(&self) -> Ordering {
        if self.is_sign_negative() {
            Ordering::Less
        } else if self.is_zero() {
            Ordering::Equal
        } else {
            Ordering::Greater
        }
    }
}

/// Format volume to short format
/// Example: 1234567 → 1.23M
pub fn format_volume(volume: u64) -> String {
    if volume == 0 {
        return "--".to_string();
    }

    #[allow(clippy::cast_precision_loss)]
    let volume_f = volume as f64;

    if volume >= 1_000_000_000 {
        format!("{:.2}B", volume_f / 1_000_000_000.0)
    } else if volume >= 1_000_000 {
        format!("{:.2}M", volume_f / 1_000_000.0)
    } else if volume >= 1_000 {
        format!("{:.2}K", volume_f / 1_000.0)
    } else {
        volume.to_string()
    }
}

/// Format decimal volume to short format (for normalized token amounts).
pub fn format_volume_decimal(volume: Decimal) -> String {
    if volume == Decimal::ZERO {
        return "--".to_string();
    }

    let volume = volume.abs();
    let billion = Decimal::from(1_000_000_000u64);
    let million = Decimal::from(1_000_000u64);
    let thousand = Decimal::from(1_000u64);

    if volume >= billion {
        format!("{}B", (volume / billion).round_dp(2))
    } else if volume >= million {
        format!("{}M", (volume / million).round_dp(2))
    } else if volume >= thousand {
        format!("{}K", (volume / thousand).round_dp(2))
    } else {
        let precision = if volume < Decimal::ONE { 6 } else { 2 };
        volume.round_dp(precision).normalize().to_string()
    }
}
