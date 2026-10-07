# UbU session planning worker

The default stdlib-only path echoes framed request/reference pairs inside an
owned session. P1B-71 also adds an optional, CPU-only torch Stage 1 and a bounded
framework probe. The original one-shot advisory scaffold is superseded.

Default installation and tests require no CUDA, GPU hardware, or torch. The
optional `torch` extra pins the CPU-only framework used by P1B-71.

## P1B-70 session worker (preserved echo path)

Run the repository module ubu_planning_worker.main with gpu-advisory/src on
PYTHONPATH. The stdlib-only worker reads bounded four-byte big-endian length
prefixes, remains alive across request/reference pairs and emits one final
response per pair. Each input carries one semantic object, correlated by
request_id; cancellation is process management. It performs no planning and
imports neither torch nor numpy on the echo path. CPU provenance remains CPU provenance.

The optional P1B-71 path uses the pinned torch extra only on the CPU device.
Stage 1's atomic tensors and exact CPU goldens are independent of the echo;
missing or broken torch retains CPU fallback. Checks install nothing. See
ubu-devshell/docs/STAGE1_WORKER.md for the approved internal-envelope scope,
installation command, dtypes, split rejection and repair fallback.

The pure Python codec uses the same golden bytes as Rust. Pytest exercises
split reads, malformed lengths, frame ordering, duplicate terminals,
cancellation and persistence through in-memory streams; it spawns no process.
The real owned-child tests are section D's Rust tests and section G's suite.
The optional extra is CPU-only; no runtime dependency is added to the stdlib echo.
A documented install is verified when a run under it is quiet, not when it
resolves. Select the installed interpreter with UBU_WORKER_TEST_PYTHON and run
ubu-devshell/scripts/check-planning-worker.sh. Its existing owned environment
probe reports import_warning_count and the owned invocation check requires zero
before accepting availability. Warnings fail even on a later import fallback;
missing Python/torch explicitly skips installation verification. Checks install
nothing. The actual installation commands live beside this rule in
ubu-devshell/docs/STAGE1_WORKER.md; this repository has no docs directory.
