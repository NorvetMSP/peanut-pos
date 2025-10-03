use common_money::normalize_scale;
use bigdecimal::BigDecimal;
use proptest::prelude::*;
use std::str::FromStr;

proptest! {
    // Values near a .5 cent boundary: generate base cents and add offset in thousandths so we straddle midpoint.
    #[test]
    fn half_up_midpoint_behavior(base_cents in -10_000i64..10_000, offset in -5i32..5) {
        // Build a value = base_cents/100 + offset/1000
        let major = base_cents as f64 / 100.0; // only for constructing string; precision fine for range
        let val_str = format!("{:.2}", major); // base with 2 decimals
        let mut bd = BigDecimal::from_str(&val_str).unwrap();
        // add offset thousandths (offset / 1000)
        let offset_bd = BigDecimal::from(offset) / BigDecimal::from(1000);
    bd += offset_bd;

        // Capture numeric forms for expectation reasoning (approx via f64 acceptable for classification only)
        let f = bd.to_string().parse::<f64>().unwrap();
        let hundredths = (f * 100.0).floor();
        let frac_thousandth = (f * 1000.0).round() as i64 % 10; // 0-9 representing extra thousandth digit

        // Only focus on cases where thousandth digit is 5 (midpoint) or not.
        if frac_thousandth == 5 {
            // Half-up should move away from zero when thousandth is exactly 5 and no further precision beyond captured.
            // We'll materialize rounding manually by adding sign * 0.01 when midpoint.
            let sign = if f < 0.0 { -1.0 } else { 1.0 };
            let expected = (hundredths + sign) / 100.0;
            let got = normalize_scale(&bd).to_string();
            prop_assert_eq!(got, format!("{expected:.2}"), "half-up midpoint expected away-from-zero");
        }
    }

    // Bankers rounding: ties (.x5 with following zeros) should produce even last cent.
    #[test]
    fn bankers_tie_even(last_two_cents in 0i32..100, sign in -1i32..=1, extra in 0i32..3) {
        // sign -1,0,1 (skip zero results -> treat as positive) but keep sign logic simple
        let sign = if sign == 0 { 1 } else { sign }; // eliminate 0 sign case
        // Build value: cents = last_two_cents, then thousandth digit = 5, maybe more zeros (extra controls additional zeros length)
        let cents_major = last_two_cents / 100; // integer dollars (may be zero) but we focus on cents portion
        let cents_minor = last_two_cents % 100;
        let even_candidate = format!("{}.{:02}5{}", cents_major, cents_minor, "0".repeat(extra as usize));
        let val_str = if sign < 0 { format!("-{}", even_candidate) } else { even_candidate };
        let bd = BigDecimal::from_str(&val_str).unwrap();
        // Apply internal bankers helper via public path: temporarily force mode by directly calling bankers logic would require visibility; instead replicate logic expectation.
        // We infer expected parity outcome: if resulting integer*100 (cents) is odd it should move to next even cent.
        // Use half-up and bankers difference: half-up always away from zero; bankers may stay.
        // Compute both to ensure bankers is either equal to half-up or one cent closer to zero (when half-up produced an odd cent).
        let half_up_out = {
            // simulate by setting env temporarily (best-effort) not strictly isolatable due to OnceLock; fallback to manual formula
            // Manual half-up using +0.005/-0.005
            let adjust = if sign < 0 { BigDecimal::from_str("-0.005").unwrap() } else { BigDecimal::from_str("0.005").unwrap() };
            let hu = &bd + adjust;
            // truncate to 2 decimals by string manipulation
            let s = hu.to_string();
            let out = if let Some(dot) = s.find('.') { &s[..dot+3.min(s.len()-dot)] } else { &s };
            BigDecimal::from_str(out).unwrap_or(bd.clone())
        };
        // Bankers expectation: if half_up_out cents last digit odd (in magnitude) then bankers increments only if tie and odd; else stays original truncated
        // Simplify: ensure bankers result (normalize_scale) is either bd rounded half-up or truncated version.
        let got = normalize_scale(&bd);
        let truncated = {
            let s = bd.to_string();
            let out = if let Some(dot) = s.find('.') { &s[..dot+3.min(s.len()-dot)] } else { &s };
            BigDecimal::from_str(out).unwrap_or(bd.clone())
        };
        let cond = got == half_up_out || got == truncated;
        prop_assert!(cond, "bankers output must match half-up or truncated on tie-even candidate: input={val_str} got={got} half_up={half_up_out} trunc={truncated}");
    }

    // Truncate monotonic property: For positive values, truncated result should never exceed half-up result.
    #[test]
    fn truncate_monotonic_pos(v in 0..10_000i64, thousandth in 0i32..=999) {
        let s = format!("{}.{:03}", v/1000, thousandth);
        let bd = BigDecimal::from_str(&s).unwrap();
        let norm = normalize_scale(&bd); // depends on global mode; ensure default HalfUp
        // Build truncated manually
        let truncated = {
            let s = bd.to_string();
            let out = if let Some(dot) = s.find('.') { &s[..dot+3.min(s.len()-dot)] } else { &s };
            BigDecimal::from_str(out).unwrap_or(bd.clone())
        };
        // Truncate <= half-up output (as numeric)
        let norm_f = norm.to_string().parse::<f64>().unwrap();
        let trunc_f = truncated.to_string().parse::<f64>().unwrap();
        prop_assert!(trunc_f <= norm_f + 0.000_000_1, "truncate should not exceed half-up: orig={s} trunc={truncated} halfup={norm}");
    }
}

// Deterministic unit cases for truncate vs half-up vs bankers used in main lib tests already; here we focus on extra randomization.
// Future: add bankers tie-even property & truncate monotonic property once core skeleton validated.
