use criterion::{criterion_group, criterion_main, Criterion, black_box};
use bigdecimal::BigDecimal;
use common_money::{Money, aggregate_rounding_sum};
use std::str::FromStr;

// Simulate an integer-cents accumulation by storing i64 and only converting once.
fn sum_integer_cents(values: &[BigDecimal]) -> Money {
    let mut cents: i128 = 0; // use i128 for safety
    for v in values {
        // v has arbitrary scale; convert by multiplying by 100 and rounding half-up manually
        let s = v.to_string();
        let bd = BigDecimal::from_str(&s).unwrap();
        // Reuse existing normalize logic by constructing Money then extracting cents
        let m = Money::from(bd);
        cents += m.as_cents() as i128;
    }
    Money::from_cents(cents as i64)
}

fn generate_values(n: usize) -> Vec<BigDecimal> {
    // Mix values around common fractional edges
    let patterns = ["1.005", "2.675", "0.009", "3.333", "4.444", "5.555", "0.005", "9.999", "12.341", "7.500"];    
    (0..n).map(|i| BigDecimal::from_str(patterns[i % patterns.len()]).unwrap()).collect()
}

fn bench_accumulation(c: &mut Criterion) {
    let sizes = [100usize, 1_000, 10_000];
    for &n in &sizes {
        let data = generate_values(n);
        c.bench_function(&format!("accumulate_incremental_{n}"), |b| {
            b.iter(|| {
                let total: Money = data.iter().cloned().map(Money::from).sum();
                black_box(total);
            })
        });
        c.bench_function(&format!("accumulate_aggregate_{n}"), |b| {
            b.iter(|| {
                let total = aggregate_rounding_sum(&data);
                black_box(total);
            })
        });
        c.bench_function(&format!("accumulate_integer_cents_sim_{n}"), |b| {
            b.iter(|| {
                let total = sum_integer_cents(&data);
                black_box(total);
            })
        });
    }
}

criterion_group!(benches, bench_accumulation);
criterion_main!(benches);
