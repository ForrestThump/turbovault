/// Compares HNSW index query latency for:
///   - `load()` — fully deserialized into RAM (current default)
///   - `view()` — memory-mapped from disk (OS page cache handles paging)
///
/// Three index sizes: 1K, 5K, 10K vectors at 384 dims (BGE-small shape).
/// Each bench group runs warm: Criterion pre-warms iterations, so the mmap
/// numbers reflect steady-state page-cache performance, not cold-start.
/// A separate "cold open" group benchmarks how long load() vs view() take to
/// open an index file so you can evaluate a "load-on-demand" strategy.
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use tempfile::TempDir;
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

const DIMS: usize = 384;

fn make_index(n: usize, dir: &TempDir) -> std::path::PathBuf {
    let options = IndexOptions {
        dimensions: DIMS,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F16,
        connectivity: 0,
        expansion_add: 0,
        expansion_search: 0,
        multi: false,
    };
    let idx = Index::new(&options).unwrap();
    idx.reserve(n).unwrap();

    // Deterministic pseudo-random vectors (xorshift).
    let mut state: u64 = 0xdeadbeef_cafebabe;
    let mut next_f32 = || -> f32 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        // Map to [-1, 1]
        (state as i64 as f32) / (i64::MAX as f32)
    };

    for id in 0..n as u64 {
        let vec: Vec<f32> = (0..DIMS).map(|_| next_f32()).collect();
        idx.add(id, &vec).unwrap();
    }

    let path = dir.path().join(format!("hnsw_{n}.idx"));
    let path_str = path.to_string_lossy();
    idx.save(&path_str).unwrap();
    path
}

fn query_vec(seed: u64) -> Vec<f32> {
    let mut state = seed;
    (0..DIMS)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as i64 as f32) / (i64::MAX as f32)
        })
        .collect()
}

fn bench_search(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let sizes = [1_000usize, 5_000, 10_000];

    // Build and persist all index files up front.
    let paths: Vec<_> = sizes.iter().map(|&n| make_index(n, &dir)).collect();

    let mut group = c.benchmark_group("search_latency");
    group.sample_size(200);

    for (&n, path) in sizes.iter().zip(paths.iter()) {
        let path_str = path.to_string_lossy().to_string();

        // ── RAM: load() ──────────────────────────────────────────────────────
        let opts = IndexOptions {
            dimensions: DIMS,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F16,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };
        let ram_idx = Index::new(&opts).unwrap();
        ram_idx.load(&path_str).unwrap();

        group.bench_with_input(BenchmarkId::new("load_ram", n), &n, |b, _| {
            let q = query_vec(0xabcd1234);
            b.iter(|| {
                let results = ram_idx.search(black_box(&q), 10).unwrap();
                black_box(results)
            });
        });

        // ── Mmap: view() ─────────────────────────────────────────────────────
        let mmap_idx = Index::new(&opts).unwrap();
        mmap_idx.view(&path_str).unwrap();

        group.bench_with_input(BenchmarkId::new("view_mmap", n), &n, |b, _| {
            let q = query_vec(0xabcd1234);
            b.iter(|| {
                let results = mmap_idx.search(black_box(&q), 10).unwrap();
                black_box(results)
            });
        });
    }

    group.finish();
}

fn bench_open(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let sizes = [1_000usize, 5_000, 10_000];
    let paths: Vec<_> = sizes.iter().map(|&n| make_index(n, &dir)).collect();

    let mut group = c.benchmark_group("open_latency");
    // Fewer samples — each iteration reopens the index from disk.
    group.sample_size(50);

    let make_opts = || IndexOptions {
        dimensions: DIMS,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F16,
        connectivity: 0,
        expansion_add: 0,
        expansion_search: 0,
        multi: false,
    };

    for (&n, path) in sizes.iter().zip(paths.iter()) {
        let path_str = path.to_string_lossy().to_string();

        group.bench_with_input(BenchmarkId::new("load_open", n), &n, |b, _| {
            b.iter(|| {
                let idx = Index::new(&make_opts()).unwrap();
                idx.load(black_box(&path_str)).unwrap();
                black_box(idx)
            });
        });

        group.bench_with_input(BenchmarkId::new("view_open", n), &n, |b, _| {
            b.iter(|| {
                let idx = Index::new(&make_opts()).unwrap();
                idx.view(black_box(&path_str)).unwrap();
                black_box(idx)
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_search, bench_open);
criterion_main!(benches);
