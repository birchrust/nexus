use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use nexus_decimal::Decimal;

type D64 = Decimal<i64, 8>;
type D96 = Decimal<i128, 12>;

// ============================================================================
// FromStr
// ============================================================================

fn bench_from_str(c: &mut Criterion) {
    let mut group = c.benchmark_group("from_str");

    let inputs = [
        ("integer", "12345"),
        ("short_frac", "123.45"),
        ("full_d64", "123.45678901"),
        ("negative", "-9999.99999999"),
        ("small", "0.00000001"),
    ];

    for (name, input) in inputs {
        group.throughput(Throughput::Elements(1));
        group.bench_with_input(BenchmarkId::new("D64_exact", name), &input, |b, &s| {
            b.iter(|| std::hint::black_box(D64::from_str_exact(s)));
        });
    }

    group.bench_function("D64_lossy_excess", |b| {
        b.iter(|| std::hint::black_box(D64::from_str_lossy("123.4567890123456")));
    });

    group.bench_function("D96_exact", |b| {
        b.iter(|| std::hint::black_box(D96::from_str_exact("123.456789012345")));
    });

    group.finish();
}

// ============================================================================
// Display
// ============================================================================

fn bench_display(c: &mut Criterion) {
    let mut group = c.benchmark_group("display");
    group.throughput(Throughput::Elements(1));

    group.bench_function("D64_integer", |b| {
        let d = D64::new(12345, 0);
        b.iter(|| {
            use std::fmt::Write;
            let mut buf = String::with_capacity(32);
            write!(buf, "{}", std::hint::black_box(d)).unwrap();
            std::hint::black_box(buf);
        });
    });

    group.bench_function("D64_fractional", |b| {
        let d = D64::new(123, 45_678_900);
        b.iter(|| {
            use std::fmt::Write;
            let mut buf = String::with_capacity(32);
            write!(buf, "{}", std::hint::black_box(d)).unwrap();
            std::hint::black_box(buf);
        });
    });

    group.finish();
}

// ============================================================================
// Main
// ============================================================================

criterion_group!(benches, bench_from_str, bench_display);
criterion_main!(benches);
