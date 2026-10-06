# UbU GPU Advisory No-Op

This Python package is a stdlib-only no-op process scaffold. It reads one JSON advisory request from stdin and writes one JSON advisory response to stdout.

Default installation and tests must not require CUDA, GPU hardware, or torch. A future torch integration may be added under the optional `torch` extra.

## P1B-70 session worker

Run the repository module ubu_planning_worker.main with gpu-advisory/src on
PYTHONPATH. The stdlib-only worker reads bounded four-byte big-endian length
prefixes, remains alive across request/reference pairs and emits one final
response per pair. Each input carries one semantic object, correlated by
request_id; cancellation is process management. It performs no planning and
imports neither torch nor numpy. CPU provenance remains CPU provenance.

The pure Python codec uses the same golden bytes as Rust. Pytest exercises
split reads, malformed lengths, frame ordering, duplicate terminals,
cancellation and persistence through in-memory streams; it spawns no process.
The real owned-child tests are section D's Rust tests and section G's suite.
The torch extra remains an explicitly unassigned later-stage TODO, after
P1B-70; this ticket installs only pytest in a local test environment under the
operator's one-time exception. No runtime dependency is added.
