//! FrameWriter benchmarks — outbound path: encode → wire bytes.
//!
//! Run with:
//!   cargo bench -p nexus-net --bench frame_writer

use criterion::{
    black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput,
};
use nexus_net::ws::{FrameWriter, Role};
use nexus_net::buf::WriteBuf;

fn bench_encode_text_server(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_text_server");
    for size in [32, 128, 512, 2048, 4096] {
        let writer = FrameWriter::new(Role::Server);
        let payload = vec![b'x'; size];
        let mut dst = vec![0u8; writer.max_encoded_len(size)];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &payload,
            |b, payload| {
                b.iter(|| {
                    let n = writer.encode_text(payload, &mut dst);
                    black_box(n);
                });
            },
        );
    }
    group.finish();
}

fn bench_encode_text_client(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_text_client");
    for size in [32, 128, 512, 2048, 4096] {
        let writer = FrameWriter::new(Role::Client);
        let payload = vec![b'x'; size];
        let mut dst = vec![0u8; writer.max_encoded_len(size)];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &payload,
            |b, payload| {
                b.iter(|| {
                    let n = writer.encode_text(payload, &mut dst);
                    black_box(n);
                });
            },
        );
    }
    group.finish();
}

fn bench_encode_into_writebuf(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_into_writebuf");
    for size in [32, 128, 512, 2048, 4096] {
        let writer = FrameWriter::new(Role::Server);
        let payload = vec![b'x'; size];
        let mut wbuf = WriteBuf::new(size + 14, 14);
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(size),
            &payload,
            |b, payload| {
                b.iter(|| {
                    writer.encode_text_into(payload, &mut wbuf);
                    black_box(wbuf.data());
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    encode,
    bench_encode_text_server,
    bench_encode_text_client,
    bench_encode_into_writebuf,
);

criterion_main!(encode);
