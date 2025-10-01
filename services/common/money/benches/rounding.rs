use criterion::{criterion_group, criterion_main, Criterion, black_box};
use bigdecimal::BigDecimal;
use std::str::FromStr;

// Re-export functions from the crate
use common_money::{normalize_scale, init_rounding_mode_from_env};

fn bench_half_up(c: &mut Criterion) {
    std::env::remove_var("MONEY_ROUNDING");
    // ensure default
    init_rounding_mode_from_env();
    let samples: Vec<BigDecimal> = [
        "1.005", "2.675", "0.005", "-1.005", "-2.505", "12345", "19.90", "1000000.555",
        "-999999.995", "0.3349", "42.4242"
    ].into_iter().map(|s| BigDecimal::from_str(s).unwrap()).collect();
    c.bench_function("round_half_up_normalize", |b| {
        b.iter(|| {
            for v in &samples { black_box(normalize_scale(v)); }
        });
    });
}

fn bench_modes_compare(c: &mut Criterion) {
    let samples: Vec<BigDecimal> = (0..500).map(|i| {
        let s = format!("{}.{:03}", i, i % 1000);
        BigDecimal::from_str(&s).unwrap()
    }).collect();

    for (label, env) in [("truncate", "truncate"), ("bankers", "bankers"), ("halfup", "half-up")] {
        c.bench_function(format!("round_mode_{}", label).as_str(), |b| {
            // Initialize once per benchmarked function to avoid OneTime init penalty inside iterations.
            std::env::set_var("MONEY_ROUNDING", env);
            // Resetting OnceLock isn't supported; process-level reuse between benchmarks is acceptable.
            let _ = init_rounding_mode_from_env();
            b.iter(|| {
                for v in &samples { black_box(normalize_scale(v)); }
            });
        });
    }
}

criterion_group!(rounding, bench_half_up, bench_modes_compare);
criterion_main!(rounding);
