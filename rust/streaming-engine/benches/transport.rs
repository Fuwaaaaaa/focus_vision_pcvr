use criterion::{criterion_group, criterion_main, Criterion, black_box};

fn bench_rtp_packetize_p_frame(c: &mut Criterion) {
    use streaming_engine::transport::rtp::RtpPacketizer;

    let mut packetizer = RtpPacketizer::new(0x46565000);
    let frame = vec![0xABu8; 50_000]; // 50KB P-frame

    c.bench_function("rtp_packetize_50kb", |b| {
        b.iter(|| {
            let packets = packetizer.packetize(black_box(&frame), 0, 1000, false);
            packetizer.recycle(packets);
        })
    });
}

fn bench_rtp_packetize_idr_frame(c: &mut Criterion) {
    use streaming_engine::transport::rtp::RtpPacketizer;

    let mut packetizer = RtpPacketizer::new(0x46565000);
    let frame = vec![0xCDu8; 200_000]; // 200KB IDR frame

    c.bench_function("rtp_packetize_200kb_idr", |b| {
        b.iter(|| {
            let packets = packetizer.packetize(black_box(&frame), 0, 1000, true);
            packetizer.recycle(packets);
        })
    });
}

fn bench_fec_encode(c: &mut Criterion) {
    use streaming_engine::transport::fec::FecEncoder;

    let mut encoder = FecEncoder::new(0.20); // 20% redundancy

    c.bench_function("fec_encode_20_shards", |b| {
        b.iter(|| {
            // Must create fresh shards each iteration (encode takes ownership)
            let shards: Vec<Vec<u8>> = (0..20).map(|_| vec![0xEF; 1400]).collect();
            let _ = encoder.encode(black_box(shards));
        })
    });
}

fn bench_fec_encode_large(c: &mut Criterion) {
    use streaming_engine::transport::fec::FecEncoder;

    let mut encoder = FecEncoder::new(0.20);

    c.bench_function("fec_encode_100_shards", |b| {
        b.iter(|| {
            let shards: Vec<Vec<u8>> = (0..100).map(|_| vec![0xEF; 1400]).collect();
            let _ = encoder.encode(black_box(shards));
        })
    });
}

fn bench_adaptive_fec_adjust(c: &mut Criterion) {
    use streaming_engine::transport::fec::AdaptiveFecController;

    c.bench_function("adaptive_fec_adjust_cycle", |b| {
        b.iter(|| {
            let mut ctrl = AdaptiveFecController::new(0.05, 0.40, 0.20);
            // Simulate a sequence of loss rate changes
            for loss in [0.005, 0.01, 0.02, 0.04, 0.06, 0.03, 0.01, 0.005] {
                ctrl.adjust(black_box(loss));
            }
            black_box(ctrl.current_redundancy());
        })
    });
}

fn bench_memory_rss_read(c: &mut Criterion) {
    use streaming_engine::metrics::memory::MemoryMonitor;

    c.bench_function("memory_rss_read", |b| {
        b.iter(|| {
            black_box(MemoryMonitor::current_rss_mb());
        })
    });
}

fn bench_config_parse_validate(c: &mut Criterion) {
    use streaming_engine::config::AppConfig;

    let toml_str = include_str!("../../../config/default.toml");

    c.bench_function("config_parse_validate", |b| {
        b.iter(|| {
            let mut cfg: AppConfig = toml::from_str(black_box(toml_str)).unwrap();
            cfg.validate();
        })
    });
}

criterion_group!(
    benches,
    bench_rtp_packetize_p_frame,
    bench_rtp_packetize_idr_frame,
    bench_fec_encode,
    bench_fec_encode_large,
    bench_adaptive_fec_adjust,
    bench_memory_rss_read,
    bench_config_parse_validate,
);
criterion_main!(benches);
