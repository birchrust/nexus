use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use nexus_decimal::Decimal;
use std::str::FromStr;

type D32 = Decimal<i32, 4>;
type D64 = Decimal<i64, 8>;
type D96 = Decimal<i128, 12>;
type D128 = Decimal<i128, 18>;

// ============================================================================
// Addition
// ============================================================================

fn bench_add(c: &mut Criterion) {
    let mut group = c.benchmark_group("cmp_add");
    group.throughput(Throughput::Elements(1));

    group.bench_function("nexus_D64", |b| {
        let a = D64::from_str_exact("123.45678901").unwrap();
        let b_val = D64::from_str_exact("89.12345678").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) + std::hint::black_box(b_val)));
    });

    group.bench_function("rust_decimal", |b| {
        let a = rust_decimal::Decimal::from_str("123.45678901").unwrap();
        let b_val = rust_decimal::Decimal::from_str("89.12345678").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) + std::hint::black_box(b_val)));
    });

    group.finish();
}

// ============================================================================
// Multiplication
// ============================================================================

fn bench_mul(c: &mut Criterion) {
    let mut group = c.benchmark_group("cmp_mul");
    group.throughput(Throughput::Elements(1));

    group.bench_function("nexus_D64", |b| {
        let a = D64::from_str_exact("123.456789").unwrap();
        let b_val = D64::from_str_exact("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b_val)));
    });

    group.bench_function("rust_decimal", |b| {
        let a = rust_decimal::Decimal::from_str("123.456789").unwrap();
        let b_val = rust_decimal::Decimal::from_str("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b_val)));
    });

    group.finish();
}

// ============================================================================
// Division
// ============================================================================

fn bench_div(c: &mut Criterion) {
    let mut group = c.benchmark_group("cmp_div");
    group.throughput(Throughput::Elements(1));

    group.bench_function("nexus_D64", |b| {
        let a = D64::from_str_exact("123.456789").unwrap();
        let b_val = D64::from_str_exact("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b_val)));
    });

    group.bench_function("rust_decimal", |b| {
        let a = rust_decimal::Decimal::from_str("123.456789").unwrap();
        let b_val = rust_decimal::Decimal::from_str("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b_val)));
    });

    group.finish();
}

// ============================================================================
// FromStr parsing
// ============================================================================

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("cmp_parse");
    group.throughput(Throughput::Elements(1));

    let input = "12345.67890123";

    group.bench_function("nexus_D64", |b| {
        b.iter(|| std::hint::black_box(D64::from_str_exact(std::hint::black_box(input))));
    });

    group.bench_function("rust_decimal", |b| {
        b.iter(|| {
            std::hint::black_box(rust_decimal::Decimal::from_str(std::hint::black_box(input)))
        });
    });

    group.finish();
}

// ============================================================================
// Display formatting
// ============================================================================

fn bench_display(c: &mut Criterion) {
    let mut group = c.benchmark_group("cmp_display");
    group.throughput(Throughput::Elements(1));

    group.bench_function("nexus_D64", |b| {
        let d = D64::from_str_exact("12345.6789").unwrap();
        b.iter(|| {
            use std::fmt::Write;
            let mut buf = String::with_capacity(32);
            write!(buf, "{}", std::hint::black_box(d)).unwrap();
            std::hint::black_box(buf);
        });
    });

    group.bench_function("rust_decimal", |b| {
        let d = rust_decimal::Decimal::from_str("12345.6789").unwrap();
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
// Trading scenario: price × quantity
// ============================================================================

fn bench_price_x_qty(c: &mut Criterion) {
    let mut group = c.benchmark_group("cmp_price_x_qty");
    group.throughput(Throughput::Elements(1));

    group.bench_function("nexus_mul_int", |b| {
        let price = D64::from_str_exact("1234.56789012").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(price).mul_int(100)));
    });

    group.bench_function("nexus_mul_decimal", |b| {
        let price = D64::from_str_exact("1234.56789012").unwrap();
        let qty = D64::from_str_exact("100.0").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(price) * std::hint::black_box(qty)));
    });

    group.bench_function("rust_decimal", |b| {
        let price = rust_decimal::Decimal::from_str("1234.56789012").unwrap();
        let qty = rust_decimal::Decimal::from_str("100.0").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(price) * std::hint::black_box(qty)));
    });

    group.finish();
}

// ============================================================================
// Cross-backing: i32 vs i64 vs i128
// ============================================================================

// ============================================================================
// vs fixdec
// ============================================================================

fn bench_vs_fixdec(c: &mut Criterion) {
    let mut group = c.benchmark_group("vs_fixdec");
    group.throughput(Throughput::Elements(1));

    // Mul
    group.bench_function("nexus_mul", |b| {
        let a = D64::from_str_exact("123.456789").unwrap();
        let b_val = D64::from_str_exact("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b_val)));
    });
    group.bench_function("fixdec_mul", |b| {
        let a = fixdec::D64::from_str("123.456789").unwrap();
        let b_val = fixdec::D64::from_str("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b_val)));
    });

    // Div
    group.bench_function("nexus_div", |b| {
        let a = D64::from_str_exact("123.456789").unwrap();
        let b_val = D64::from_str_exact("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b_val)));
    });
    group.bench_function("fixdec_div", |b| {
        let a = fixdec::D64::from_str("123.456789").unwrap();
        let b_val = fixdec::D64::from_str("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b_val)));
    });

    // Add
    group.bench_function("nexus_add", |b| {
        let a = D64::from_str_exact("123.45678901").unwrap();
        let b_val = D64::from_str_exact("89.12345678").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) + std::hint::black_box(b_val)));
    });
    group.bench_function("fixdec_add", |b| {
        let a = fixdec::D64::from_str("123.45678901").unwrap();
        let b_val = fixdec::D64::from_str("89.12345678").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) + std::hint::black_box(b_val)));
    });

    // Parse
    group.bench_function("nexus_parse", |b| {
        b.iter(|| D64::from_str_exact(std::hint::black_box("12345.67890123")));
    });
    group.bench_function("fixdec_parse", |b| {
        b.iter(|| fixdec::D64::from_str(std::hint::black_box("12345.67890123")));
    });

    group.finish();
}

fn bench_backing_mul(c: &mut Criterion) {
    let mut group = c.benchmark_group("backing_mul");
    group.throughput(Throughput::Elements(1));

    group.bench_function("i32_D4", |b| {
        let a = D32::new(12, 3456);
        let b_val = D32::new(7, 8901);
        b.iter(|| std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b_val)));
    });

    group.bench_function("i64_D8", |b| {
        let a = D64::from_str_exact("123.456789").unwrap();
        let b_val = D64::from_str_exact("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b_val)));
    });

    group.bench_function("i128_D12", |b| {
        let a = D96::from_str_exact("123.456789012").unwrap();
        let b_val = D96::from_str_exact("7.890123456").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b_val)));
    });

    group.bench_function("i128_D18", |b| {
        let a = D128::from_str_exact("123.456789012345678").unwrap();
        let b_val = D128::from_str_exact("7.890123456789012").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) * std::hint::black_box(b_val)));
    });

    group.finish();
}

fn bench_backing_div(c: &mut Criterion) {
    let mut group = c.benchmark_group("backing_div");
    group.throughput(Throughput::Elements(1));

    group.bench_function("i32_D4", |b| {
        let a = D32::new(123, 4567);
        let b_val = D32::new(7, 8901);
        b.iter(|| std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b_val)));
    });

    group.bench_function("i64_D8", |b| {
        let a = D64::from_str_exact("123.456789").unwrap();
        let b_val = D64::from_str_exact("7.890123").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b_val)));
    });

    group.bench_function("i128_D12", |b| {
        let a = D96::from_str_exact("123.456789012").unwrap();
        let b_val = D96::from_str_exact("7.890123456").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b_val)));
    });

    group.bench_function("i128_D18", |b| {
        let a = D128::from_str_exact("123.456789012345678").unwrap();
        let b_val = D128::from_str_exact("7.890123456789012").unwrap();
        b.iter(|| std::hint::black_box(std::hint::black_box(a) / std::hint::black_box(b_val)));
    });

    group.finish();
}

fn bench_backing_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("backing_parse");
    group.throughput(Throughput::Elements(1));

    group.bench_function("i32_D4", |b| {
        b.iter(|| D32::from_str_exact(std::hint::black_box("123.4567")));
    });

    group.bench_function("i64_D8", |b| {
        b.iter(|| D64::from_str_exact(std::hint::black_box("12345.67890123")));
    });

    group.bench_function("i128_D12", |b| {
        b.iter(|| D96::from_str_exact(std::hint::black_box("12345.678901234567")));
    });

    group.bench_function("i128_D18", |b| {
        b.iter(|| D128::from_str_exact(std::hint::black_box("12345.678901234567890123")));
    });

    group.finish();
}

// ============================================================================
// Main
// ============================================================================

criterion_group!(
    benches,
    bench_add,
    bench_mul,
    bench_div,
    bench_parse,
    bench_display,
    bench_price_x_qty,
    bench_vs_fixdec,
    bench_backing_mul,
    bench_backing_div,
    bench_backing_parse,
);
criterion_main!(benches);
