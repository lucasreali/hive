//! Codec throughput baseline: `cargo bench -p hive-protocol`.

use std::hint::black_box;

use bytes::BytesMut;
use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use hive_protocol::{Control, Frame, FrameCodec};
use tokio_util::codec::{Decoder, Encoder};

fn round_trip(frames: &[Frame]) {
    let mut buf = BytesMut::new();
    for frame in frames {
        let _ = FrameCodec.encode(frame.clone(), &mut buf);
    }
    while let Ok(Some(frame)) = FrameCodec.decode(&mut buf) {
        black_box(frame);
    }
}

fn codec(c: &mut Criterion) {
    let mut group = c.benchmark_group("codec");
    for size in [64, 4096, 65536] {
        let frames: Vec<Frame> = (0..64)
            .map(|i| Frame::terminal(i, vec![b'x'; size]))
            .collect();
        group.throughput(Throughput::Bytes((size * frames.len()) as u64));
        group.bench_function(format!("terminal_{size}b_x64"), |b| {
            b.iter(|| round_trip(&frames))
        });
    }
    let resize = Frame::control(
        1,
        &Control::Resize {
            cols: 120,
            rows: 40,
        },
    );
    let frames = vec![resize; 64];
    group.throughput(Throughput::Elements(frames.len() as u64));
    group.bench_function("control_resize_x64", |b| {
        b.iter(|| {
            round_trip(&frames);
            for frame in &frames {
                black_box(frame.to_control().ok());
            }
        })
    });
    group.finish();
}

criterion_group!(benches, codec);
criterion_main!(benches);
