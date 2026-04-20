# Load tests

Scripts under this directory drive the running HTTP service rather than the
in-process matcher. They are not compiled by cargo (cargo only picks up `.rs`
files directly under `benches/`), so living alongside the criterion suite is
safe.

## oha-1kib-en.sh

Target from IMPLEMENTATION_PLAN §M8 item 3: **p99 < 1 ms on a single core with
a 1 KiB English reference input.**

Recipe:

```bash
# 1. Start the server pinned to a single core.
BWS_API_KEYS="$(openssl rand -hex 24)" taskset -c 0 \
    cargo run --release --locked &
SERVER_PID=$!

# 2. Run the load test (reuse the same key you set above).
BWS_API_KEY="${BWS_API_KEYS}" ./benches/load/oha-1kib-en.sh

# 3. Tear down.
kill "${SERVER_PID}"
```

Defaults: 30 s duration, 64 concurrent workers, against
`http://127.0.0.1:8080/v1/check`. Override positionally:

```bash
BWS_API_KEY=... ./benches/load/oha-1kib-en.sh http://127.0.0.1:8080/v1/check 60s 128
```

The script fails fast if `oha` or `python3` is missing, or if `BWS_API_KEY`
isn't set. oha prints the latency histogram plus p50/p95/p99/p99.9 at the end
of the run — that's the number the milestone gate is written against.
