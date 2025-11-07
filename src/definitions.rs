//! Core domain definitions (non-API-specific)

use std::ops::{Deref, Sub};

use serde::Deserialize;

#[derive(Deserialize, Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Balance(pub u128);

impl Deref for Balance {
    type Target = u128;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Sub for Balance {
    type Output = Self;

    fn sub(self, r: Self) -> Self {
        // TODO: it's better to replace u128 with Decimal
        #[expect(clippy::arithmetic_side_effects)]
        Balance(self.0 - r.0)
    }
}

impl Balance {
    pub fn format(&self, decimals: crate::legacy_types::Decimals) -> f64 {
        #[expect(clippy::cast_precision_loss)]
        let float = **self as f64;

        float / decimal_exponent_product(decimals)
    }

    pub fn parse(float: f64, decimals: crate::legacy_types::Decimals) -> Self {
        let parsed_float = (float * decimal_exponent_product(decimals)).round();

        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        Self(parsed_float as _)
    }
}

pub fn decimal_exponent_product(decimals: crate::legacy_types::Decimals) -> f64 {
    10f64.powi(decimals.into())
}

#[cfg(test)]
#[test]
#[expect(
    clippy::inconsistent_digit_grouping,
    clippy::unreadable_literal,
    clippy::float_cmp
)]
fn balance_insufficient_precision() {
    const DECIMALS: crate::legacy_types::Decimals = 10;

    let float = 931395.862219815_3;
    let parsed = Balance::parse(float, DECIMALS);

    assert_eq!(*parsed, 931395_862219815_2);
    assert_eq!(parsed.format(DECIMALS), 931395.862219815_1);
}
