# Load tests

Scripts under this directory drive the running HTTP service rather than the
in-process matcher. They are not compiled by cargo (cargo only picks up `.rs`
files directly under `benches/`), so living alongside the criterion suite is
safe.

## scripts/load-test.sh (preferred entry point)

`../../scripts/load-test.sh` is the orchestrator — it boots the release binary
pinned to core 0 on port 18080, sanity-POSTs to confirm it's our server,
invokes `oha-1kib-en.sh`, and writes a timestamped report to
`reports/<timestamp>-<list_sha>.txt` (gitignored). Defaults to `c=1` because
that's the service-time config the §M8 gate is written against.

## oha-1kib-en.sh

Target from IMPLEMENTATION_PLAN §M8 item 3: **p99 < 1 ms on a single core with
a 1 KiB English reference input.** Measured at **c=1** — the gate is about
matcher latency, not queueing. On a 1-core-pinned server, c > 1 quickly puts
you in the queue-dominated regime where p99 reflects depth rather than
service time.

Install the pinned `oha` (version lives in the top-level `Makefile` under
`OHA_VERSION`):

```bash
make install-tools
```

Recipe:

```bash
# 1. Start the server pinned to a single core.
VV_API_KEYS="$(openssl rand -hex 24)" taskset -c 0 \
    cargo run --release --locked &
SERVER_PID=$!

# 2. Run the load test (reuse the same key you set above).
VV_API_KEY="${VV_API_KEYS}" ./benches/load/oha-1kib-en.sh

# 3. Tear down.
kill "${SERVER_PID}"
```

Defaults: 30 s duration, 64 concurrent workers, against
`http://127.0.0.1:8080/v1/check`. c=64 here is a throughput probe —
useful for saturation/queue-behavior work, not for the §M8 gate. Use
`../../scripts/load-test.sh` (which defaults to c=1) when you're
producing a release report. Override positionally:

```bash
VV_API_KEY=... ./benches/load/oha-1kib-en.sh http://127.0.0.1:8080/v1/check 60s 128
```

The script fails fast if `oha` or `python3` is missing, or if `VV_API_KEY`
isn't set. oha prints the latency histogram plus p50/p95/p99/p99.9 at the end
of the run — that's the number the milestone gate is written against.
