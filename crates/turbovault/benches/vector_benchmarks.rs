//! Semantic quality + latency benchmarks for vector search.
//!
//! Demonstrates superior retrieval quality vs TF-IDF on paraphrase/synonym queries.
//! Run with: cargo bench --features vector-search --bench vector_benchmarks
//!
//! The semantic quality comparison table is printed as a side-effect of
//! `bench_semantic_quality` (single iteration) and is also captured in the
//! Criterion HTML report.

#![cfg(feature = "vector-search")]

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::runtime::Runtime;
use turbovault_core::{ConfigProfile, VaultConfig};
use turbovault_tools::{SearchEngine, SimilarityEngine};
use turbovault_vault::VaultManager;
use turbovault_vector::{ChunkStore, FastembedEngine, IndexBuilder, SearchRouter, VectorIndex};

// ─── Test dataset ────────────────────────────────────────────────────────────

/// (filename, content) pairs organised in 5 semantic clusters (10 notes each).
/// Notes within a cluster share meaning but use varied vocabulary — exactly the
/// case where TF-IDF fails and dense vectors win.
const NOTES: &[(&str, &str)] = &[
    // Cluster: containers / zero-downtime deployments
    (
        "container_updates.md",
        "# Container Update Protocol\n\nRolling updates in Kubernetes replace pods one at a time, ensuring a configurable minimum number remain available. This guarantees zero service interruption during upgrades.",
    ),
    (
        "docker_compose.md",
        "# Docker Compose Orchestration\n\nDocker Compose files define multi-container applications. The `deploy.update_config.order` setting controls whether new replicas are started before old ones stop.",
    ),
    (
        "kubernetes_deploy.md",
        "# Kubernetes Deployment Strategies\n\nBlue-green and canary releases limit blast radius during rollouts. The recreate strategy causes brief downtime; rolling update avoids it.",
    ),
    (
        "service_mesh.md",
        "# Service Mesh Traffic Management\n\nIstio and Linkerd handle progressive delivery, routing a percentage of live traffic to newly deployed versions before full cutover.",
    ),
    (
        "helm_charts.md",
        "# Helm Chart Lifecycle\n\nHelm hooks allow pre/post-upgrade scripts. Combined with readiness probes, this enables safe automated rollouts of containerized workloads.",
    ),
    (
        "container_health.md",
        "# Container Health Checks\n\nLiveness and readiness probes prevent Kubernetes from routing traffic to pods that have not finished initializing or have entered a crash loop.",
    ),
    (
        "podman_pods.md",
        "# Podman Pod Management\n\nPodman runs daemonless containers. Pod groups share network namespace, enabling sidecar patterns identical to Kubernetes without a full cluster.",
    ),
    (
        "oci_images.md",
        "# OCI Image Layers\n\nContainer images are layered filesystems. Layer caching drastically reduces build and push times when only application code changes between releases.",
    ),
    (
        "runtime_security.md",
        "# Runtime Container Security\n\nSeccomp profiles and AppArmor policies restrict system calls available to containers, reducing the attack surface of a compromised workload.",
    ),
    (
        "registry_sync.md",
        "# Container Registry Synchronisation\n\nMirroring images to a local registry speeds up deployments in air-gapped environments and protects against upstream registry outages.",
    ),
    // Cluster: vector databases / ANN search
    (
        "hnsw_vectors.md",
        "# HNSW Vector Index\n\nHierarchical Navigable Small World graphs achieve sub-linear approximate nearest-neighbour search. The M parameter controls graph connectivity vs memory trade-off.",
    ),
    (
        "lancedb_note.md",
        "# LanceDB Columnar Vector Store\n\nLanceDB stores embeddings in columnar Arrow format alongside metadata, enabling fast filtered ANN queries without a separate metadata store.",
    ),
    (
        "pgvector.md",
        "# pgvector Extension\n\nAdding a vector column to Postgres tables and an IVFFlat or HNSW index allows similarity search to live alongside relational data in the same transaction.",
    ),
    (
        "embedding_pipeline.md",
        "# Embedding Generation Pipeline\n\nSentence transformers map variable-length text to fixed-dimension dense representations. Batching improves GPU utilisation during offline indexing.",
    ),
    (
        "cosine_sim.md",
        "# Cosine Similarity Retrieval\n\nCosine distance measures the angle between embedding vectors, making it scale-invariant. Dot-product distance is faster but requires unit-norm vectors.",
    ),
    (
        "quantisation.md",
        "# Vector Quantisation Techniques\n\nProduct quantisation compresses embeddings from 4 bytes to 1 byte per dimension, enabling billion-scale ANN search within DRAM budgets.",
    ),
    (
        "reranking.md",
        "# Cross-Encoder Reranking\n\nA two-stage retrieval pipeline uses a bi-encoder for fast recall and a cross-encoder to rerank the top-k candidates for maximum precision.",
    ),
    (
        "semantic_cache.md",
        "# Semantic Query Cache\n\nCaching embedding vectors of recent queries lets a semantic cache return results for paraphrase queries without hitting the index at all.",
    ),
    (
        "matryoshka.md",
        "# Matryoshka Representation Learning\n\nMRL trains a single embedding model whose prefix dimensions already form a good representation, allowing adaptive dimensionality reduction at query time.",
    ),
    (
        "ann_benchmarks.md",
        "# ANN Benchmark Methodology\n\nann-benchmarks measures recall@10 and queries-per-second across HNSW, IVF, and ScaNN implementations under identical hardware constraints.",
    ),
    // Cluster: DevOps / code review workflows
    (
        "pr_workflow.md",
        "# Pull Request Review Workflow\n\nA healthy PR pipeline includes automated linting, test execution, and at least one human review before merge. Draft PRs signal work in progress.",
    ),
    (
        "ci_pipeline.md",
        "# Continuous Integration Pipeline\n\nGitHub Actions and GitLab CI run build, test, and lint jobs on every push. Branch protection rules prevent merging if required checks fail.",
    ),
    (
        "gitops_argocd.md",
        "# GitOps with ArgoCD\n\nArgoCD continuously reconciles the live cluster state with the desired state declared in a Git repository, eliminating manual kubectl apply commands.",
    ),
    (
        "code_review.md",
        "# Effective Code Review Practices\n\nGood reviewers focus on logic, security, and maintainability rather than style. Inline comments with suggested edits speed up the review cycle.",
    ),
    (
        "branch_strategy.md",
        "# Git Branching Strategy\n\nTrunk-based development with short-lived feature branches reduces merge conflicts and keeps the main branch always releasable.",
    ),
    (
        "changelog_automation.md",
        "# Automated Changelog Generation\n\nConventional commits (feat, fix, chore) feed tools like semantic-release to auto-generate changelogs and bump semver on every merge to main.",
    ),
    (
        "code_owners.md",
        "# CODEOWNERS Routing\n\nCODEOWNERS files automatically assign reviewers based on file path, ensuring domain experts review every change in their area of ownership.",
    ),
    (
        "pre_commit_hooks.md",
        "# Pre-commit Hook Configuration\n\nPre-commit runs formatters and linters locally before a commit is created, catching issues before they reach CI and waste remote compute.",
    ),
    (
        "merge_queue.md",
        "# Merge Queue Safety\n\nGitHub merge queues batch PRs and test them together before merging, preventing the situation where two individually-green PRs break main when combined.",
    ),
    (
        "deploy_gate.md",
        "# Deployment Gate Controls\n\nManual approval gates in CD pipelines pause promotion to production until a human signs off, even when all automated checks pass.",
    ),
    // Cluster: personal knowledge management
    (
        "zettelkasten.md",
        "# Zettelkasten Method\n\nZettelkasten links atomic permanent notes through bidirectional references, allowing ideas to accumulate and connect across disciplines over years.",
    ),
    (
        "evergreen_notes.md",
        "# Evergreen Notes\n\nAndy Matuschak's evergreen notes are written to develop and accumulate over time rather than capturing a fleeting thought. Titles are assertions, not topics.",
    ),
    (
        "spaced_repetition.md",
        "# Spaced Repetition Systems\n\nAnki's SM-2 algorithm schedules flashcard reviews at increasing intervals, exploiting the spacing effect to maximise long-term retention per review minute.",
    ),
    (
        "progressive_summarisation.md",
        "# Progressive Summarisation\n\nTiago Forte's technique layers highlights and bold on existing notes to distil the most valuable content without rewriting the original.",
    ),
    (
        "pkm_workflows.md",
        "# Personal Knowledge Management Workflows\n\nPARA (Projects, Areas, Resources, Archives) organises information by actionability, making retrieval efficient for active work.",
    ),
    (
        "interstitial_journaling.md",
        "# Interstitial Journaling\n\nWriting a sentence or two between tasks captures context switches and emotional state, creating a searchable record of a working day.",
    ),
    (
        "concept_maps.md",
        "# Concept Map Construction\n\nDrawing explicit relationships between ideas as nodes and labelled edges reveals structural gaps and hidden connections in existing knowledge.",
    ),
    (
        "note_linking.md",
        "# Bidirectional Note Linking\n\nWikilinks create associations between ideas. A backlink list answers 'what do I already know that relates to this?' without re-reading everything.",
    ),
    (
        "writing_inbox.md",
        "# Writing Inbox Workflow\n\nCapturing every fleeting idea into a single inbox then triaging into the appropriate location prevents both loss of ideas and premature organisation.",
    ),
    (
        "literature_notes.md",
        "# Literature Notes vs Permanent Notes\n\nLiterature notes record what a source says; permanent notes record what you think about it and how it connects to existing ideas.",
    ),
    // Cluster: cryptography / secure channels
    (
        "pubkey_crypto.md",
        "# Public Key Cryptography\n\nRSA and elliptic-curve algorithms use a mathematically linked key pair: the public key encrypts or verifies; the private key decrypts or signs.",
    ),
    (
        "tls_handshake.md",
        "# TLS Handshake Protocol\n\nTLS 1.3 completes a full handshake in one round trip. The client sends supported cipher suites; the server selects one and presents its certificate.",
    ),
    (
        "key_derivation.md",
        "# Key Derivation Functions\n\nArgon2 and scrypt stretch a low-entropy password into a high-entropy key by consuming memory and CPU, making brute-force attacks expensive.",
    ),
    (
        "certificate_authority.md",
        "# Certificate Authority Hierarchy\n\nA root CA signs intermediate CAs, which sign end-entity certificates. This chain of trust allows revocation at the intermediate level without rotating the root.",
    ),
    (
        "diffie_hellman.md",
        "# Diffie-Hellman Key Exchange\n\nDH allows two parties to derive a shared secret over an insecure channel without transmitting the secret itself, forming the basis of forward-secret TLS.",
    ),
    (
        "jwt_auth.md",
        "# JWT Authentication\n\nJSON Web Tokens encode claims as base64url JSON and are signed with HMAC-SHA256 or RSA. The signature prevents tampering without server-side session state.",
    ),
    (
        "zero_knowledge.md",
        "# Zero-Knowledge Proofs\n\nzk-SNARKs let a prover convince a verifier that a statement is true without revealing any information beyond the truth of the statement itself.",
    ),
    (
        "secure_enclave.md",
        "# Secure Enclave Attestation\n\nApple's Secure Enclave and Intel SGX provide hardware-isolated execution environments whose integrity can be remotely attested via cryptographic proofs.",
    ),
    (
        "password_hashing.md",
        "# Password Hashing Best Practices\n\nPasswords must be hashed with bcrypt, scrypt, or Argon2 — never SHA-256 alone. Each hash must include a unique per-user salt to prevent rainbow table attacks.",
    ),
    (
        "encrypted_transit.md",
        "# Encrypting Data in Transit\n\nEnd-to-end encryption ensures only the communicating parties can read messages. Signal Protocol achieves this using the Double Ratchet algorithm for forward secrecy.",
    ),
];

/// Queries with ground truth: (query, expected_top_note, cluster_description)
const QUERIES: &[(&str, &str, &str)] = &[
    (
        "automatically update running containers without downtime",
        "container_updates.md",
        "Container Update Protocol",
    ),
    (
        "embedding similarity nearest neighbor search",
        "hnsw_vectors.md",
        "HNSW Vector Index",
    ),
    (
        "code review pull request validation pipeline",
        "pr_workflow.md",
        "Pull Request Review Workflow",
    ),
    (
        "linking ideas across permanent notes",
        "zettelkasten.md",
        "Zettelkasten Method",
    ),
    (
        "asymmetric key exchange for secure channels",
        "pubkey_crypto.md",
        "Public Key Cryptography",
    ),
    (
        "progressive delivery canary traffic routing",
        "service_mesh.md",
        "Service Mesh Traffic Management",
    ),
    (
        "columnar storage dense vector retrieval",
        "lancedb_note.md",
        "LanceDB Columnar Vector Store",
    ),
    (
        "highlight distillation across existing notes",
        "progressive_summarisation.md",
        "Progressive Summarisation",
    ),
    (
        "memory-hard password stretching function",
        "key_derivation.md",
        "Key Derivation Functions",
    ),
    (
        "automated test execution on every commit merge protection",
        "ci_pipeline.md",
        "Continuous Integration Pipeline",
    ),
];

// ─── Vault setup ─────────────────────────────────────────────────────────────

async fn setup_semantic_vault() -> (TempDir, Arc<VaultManager>) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let vault_path = temp_dir.path();

    for (name, content) in NOTES {
        tokio::fs::write(vault_path.join(name), content)
            .await
            .expect("Failed to write note");
    }

    let mut config = ConfigProfile::Development.create_config();
    let vault_config = VaultConfig::builder("bench", vault_path)
        .build()
        .expect("Failed to create vault config");
    config.vaults.push(vault_config);

    let manager = VaultManager::new(config).expect("Failed to create vault manager");
    manager
        .initialize()
        .await
        .expect("Failed to initialize vault");

    (temp_dir, Arc::new(manager))
}

async fn setup_vector_infra(
    vault_path: &std::path::Path,
) -> (
    Arc<SearchRouter>,
    Arc<tokio::sync::RwLock<VectorIndex>>,
    Arc<ChunkStore>,
) {
    let vector_dir = vault_path.join(".turbovault").join("vectors");
    tokio::fs::create_dir_all(&vector_dir).await.unwrap();

    let chunks = Arc::new(ChunkStore::open(&vector_dir.join("state.db")).unwrap());
    let embedder: Arc<dyn turbovault_vector::EmbeddingEngine> =
        Arc::new(FastembedEngine::new("bge-small-en-v1.5", None).unwrap());
    let dims = embedder.dimensions();

    let index = Arc::new(tokio::sync::RwLock::new(
        VectorIndex::open_or_create(&vector_dir.join("hnsw.idx"), dims, "f16").unwrap(),
    ));

    let builder = IndexBuilder::new(embedder.clone(), chunks.clone(), 800, 100);
    {
        let mut idx = index.write().await;
        builder.full_rebuild(vault_path, &mut *idx).await.unwrap();
    }

    let vc = turbovault_core::VectorSearchConfig::default();
    let router = Arc::new(SearchRouter::new(
        index.clone(),
        embedder,
        chunks.clone(),
        vc.rrf_k,
        vc.bm25_weight,
    ));

    (router, index, chunks)
}

// ─── Quality comparison ───────────────────────────────────────────────────────

struct QueryMetrics {
    method: &'static str,
    top1_hits: usize,
    top3_hits: usize,
    reciprocal_rank_sum: f64,
    rows: Vec<String>,
}

fn find_rank(results: &[&str], expected: &str) -> Option<usize> {
    results
        .iter()
        .position(|p| p.contains(expected))
        .map(|i| i + 1)
}

fn bench_semantic_quality(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (_temp, manager) = rt.block_on(setup_semantic_vault());
    let vault_path = manager.vault_path().clone();

    let (router, _, _) = rt.block_on(setup_vector_infra(&vault_path));
    let tfidf = rt
        .block_on(SimilarityEngine::new(manager.clone()))
        .expect("SimilarityEngine failed");
    let bm25 = rt
        .block_on(SearchEngine::new(manager.clone()))
        .expect("SearchEngine failed");

    let mut group = c.benchmark_group("semantic_quality");
    group.sample_size(10);

    // Single-shot quality evaluation (not timing, just quality table)
    group.bench_function("quality_comparison", |b| {
        b.to_async(&rt).iter(|| async {
            let mut tfidf_m = QueryMetrics {
                method: "TF-IDF",
                top1_hits: 0,
                top3_hits: 0,
                reciprocal_rank_sum: 0.0,
                rows: Vec::new(),
            };
            let mut vector_m = QueryMetrics {
                method: "Vector (hybrid)",
                top1_hits: 0,
                top3_hits: 0,
                reciprocal_rank_sum: 0.0,
                rows: Vec::new(),
            };

            for (query, expected_file, _label) in QUERIES {
                // TF-IDF
                let tfidf_results = tfidf.semantic_search(query, 10);
                let tfidf_paths: Vec<&str> =
                    tfidf_results.iter().map(|r| r.path.as_str()).collect();
                let tfidf_rank = find_rank(&tfidf_paths, expected_file);
                if tfidf_rank == Some(1) {
                    tfidf_m.top1_hits += 1;
                }
                if tfidf_rank.map_or(false, |r| r <= 3) {
                    tfidf_m.top3_hits += 1;
                }
                tfidf_m.reciprocal_rank_sum += tfidf_rank.map_or(0.0, |r| 1.0 / r as f64);

                // Vector (hybrid)
                let bm25_results = black_box(bm25.search(query).await.unwrap_or_default());
                let bm25_pairs: Vec<(String, f32)> = bm25_results
                    .iter()
                    .map(|r| (r.path.clone(), r.score as f32))
                    .collect();
                let vec_results = router
                    .hybrid_search(query, 10, bm25_pairs)
                    .await
                    .unwrap_or_default();
                let vec_paths: Vec<&str> =
                    vec_results.iter().map(|r| r.note_path.as_str()).collect();
                let vec_rank = find_rank(&vec_paths, expected_file);
                if vec_rank == Some(1) {
                    vector_m.top1_hits += 1;
                }
                if vec_rank.map_or(false, |r| r <= 3) {
                    vector_m.top3_hits += 1;
                }
                vector_m.reciprocal_rank_sum += vec_rank.map_or(0.0, |r| 1.0 / r as f64);

                tfidf_m.rows.push(format!(
                    "{:<45} | rank {:>2} | rank {:>2}",
                    &query[..query.len().min(44)],
                    tfidf_rank.map_or("—".to_string(), |r| r.to_string()),
                    vec_rank.map_or("—".to_string(), |r| r.to_string()),
                ));
            }

            let n = QUERIES.len() as f64;
            println!("\n{}", "═".repeat(80));
            println!("  Semantic Search Quality: TF-IDF vs Vector (hybrid RRF)");
            println!("{}", "═".repeat(80));
            println!(
                "{:<45} | {:^8} | {:^12}",
                "Query (truncated)", "TF-IDF", "Vector/Hybrid"
            );
            println!("{}", "─".repeat(80));
            for row in &tfidf_m.rows {
                println!("{row}");
            }
            println!("{}", "─".repeat(80));
            println!(
                "{:<45} | {:>7.1}%  | {:>11.1}%",
                "Top-1 Accuracy",
                tfidf_m.top1_hits as f64 / n * 100.0,
                vector_m.top1_hits as f64 / n * 100.0
            );
            println!(
                "{:<45} | {:>7.1}%  | {:>11.1}%",
                "Top-3 Accuracy",
                tfidf_m.top3_hits as f64 / n * 100.0,
                vector_m.top3_hits as f64 / n * 100.0
            );
            println!(
                "{:<45} | {:>8.3} | {:>12.3}",
                "MRR",
                tfidf_m.reciprocal_rank_sum / n,
                vector_m.reciprocal_rank_sum / n
            );
            println!("{}", "═".repeat(80));
        })
    });

    group.finish();
}

// ─── Latency benchmarks ───────────────────────────────────────────────────────

fn bench_vector_query(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (_temp, manager) = rt.block_on(setup_semantic_vault());
    let vault_path = manager.vault_path().clone();
    let (router, _, _) = rt.block_on(setup_vector_infra(&vault_path));

    c.bench_function("vector_query_warm", |b| {
        b.to_async(&rt).iter(|| async {
            router
                .vector_only(black_box("embedding similarity nearest neighbor"), 10)
                .await
                .unwrap()
        })
    });
}

fn bench_tfidf_query(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (_temp, manager) = rt.block_on(setup_semantic_vault());
    let engine = rt
        .block_on(SimilarityEngine::new(manager))
        .expect("SimilarityEngine failed");

    c.bench_function("tfidf_semantic_search_warm", |b| {
        b.iter(|| engine.semantic_search(black_box("embedding similarity nearest neighbor"), 10))
    });
}

fn bench_hybrid_query(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let (_temp, manager) = rt.block_on(setup_semantic_vault());
    let vault_path = manager.vault_path().clone();
    let (router, _, _) = rt.block_on(setup_vector_infra(&vault_path));
    let bm25 = rt
        .block_on(SearchEngine::new(manager))
        .expect("SearchEngine failed");

    c.bench_function("hybrid_query_warm", |b| {
        b.to_async(&rt).iter(|| async {
            let bm25_results = bm25
                .search(black_box("embedding similarity nearest neighbor"))
                .await
                .unwrap_or_default();
            let pairs: Vec<(String, f32)> = bm25_results
                .iter()
                .map(|r| (r.path.clone(), r.score as f32))
                .collect();
            router
                .hybrid_search(
                    black_box("embedding similarity nearest neighbor"),
                    10,
                    black_box(pairs),
                )
                .await
                .unwrap()
        })
    });
}

fn bench_index_build(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("index_build");
    group.sample_size(10);

    for note_count in [10usize, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("notes", note_count),
            note_count,
            |b, &n| {
                b.to_async(&rt).iter(|| async move {
                    let temp = TempDir::new().unwrap();
                    let vault_path = temp.path();

                    // Write n notes from our dataset (cycling)
                    for i in 0..n {
                        let (name, content) = NOTES[i % NOTES.len()];
                        let unique_name = format!("note_{i}_{name}");
                        tokio::fs::write(vault_path.join(unique_name), content)
                            .await
                            .unwrap();
                    }

                    let vector_dir = vault_path.join(".turbovault").join("vectors");
                    tokio::fs::create_dir_all(&vector_dir).await.unwrap();

                    let chunks = Arc::new(ChunkStore::open(&vector_dir.join("state.db")).unwrap());
                    let embedder: Arc<dyn turbovault_vector::EmbeddingEngine> =
                        Arc::new(FastembedEngine::new("bge-small-en-v1.5", None).unwrap());
                    let dims = embedder.dimensions();
                    let mut index =
                        VectorIndex::open_or_create(&vector_dir.join("hnsw.idx"), dims, "f16")
                            .unwrap();
                    let builder = IndexBuilder::new(embedder, chunks, 800, 100);

                    black_box(builder.full_rebuild(vault_path, &mut index).await.unwrap());
                    drop(temp);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    vector_benches,
    bench_index_build,
    bench_vector_query,
    bench_tfidf_query,
    bench_hybrid_query,
    bench_semantic_quality,
);
criterion_main!(vector_benches);
