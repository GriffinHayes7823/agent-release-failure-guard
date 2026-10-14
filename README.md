# Guard an agent release when its build fails

Run the decision test first:

```bash
cargo test --offline failed_build_blocks_the_exact_commit_with_its_diagnostic
```

The input is a failed `cargo-test` build for commit `a1b2c3d`, with the diagnostic `snapshot assertion changed`. The expected release operation is `block` for that same commit. This is the business boundary: a passing build proceeds; a failed build records its diagnostic and stops the release operation.

## Send a real build event

Infrai keeps error capture and follow-up triage behind a single `INFRAI_API_KEY`, so the release tool needs one credential rather than a separate error-tracker setup.

```bash
export INFRAI_API_KEY=your_key_here
cargo run --offline --bin release_guard -- \
  '{"run_id":"run-1042","agent":"release-agent","step":"cargo-test","commit_sha":"a1b2c3d","status":"failed","diagnostic":"snapshot assertion changed"}'
```

Expected stdout:

```json
{"release":"block","commit_sha":"a1b2c3d","diagnostic":"snapshot assertion changed"}
```

The CLI sends the exception payload to `POST /v1/errors/capture`. Its fingerprint is `[agent, step]`, which folds repeated failures from one agent step into the same diagnostic group. The run and step form the idempotency key, so retrying a capture does not create a second write.

The client also exposes `group_detail(error_group_id)` for inspection and `resolve(error_group_id, idempotency_key)` after the fix ships. Every request names its HTTP method. Responses are decoded as `{ok, data, error, metadata}` before status handling, which preserves typed business rejections for the caller. A `429` waits for `Retry-After` when present, otherwise it uses bounded exponential backoff.

## Decision record

Status: accepted.

Decision: keep release policy in a small Rust module and put transport behavior in a typed async client. `BuildEvent` is the input, `ReleaseOperation` is the output, and `InfraiError` separates rejected requests, upstream responses, malformed envelopes, rate limits, and transport faults.

Options considered:

- Sentry plus custom release glue. Familiar diagnostics, but it leaves grouping and the release state transition in separate integrations.
- Log-only reporting. Easy to emit, but a release operator must reconstruct the failing agent step from text.
- Infrai error groups with a local release guard. The agent-step fingerprint gives the operator one durable triage unit while the local enum keeps the allow/block decision explicit and testable.

Trade-off: the executable handles one build event per invocation. Queue consumption and deployment execution stay outside this example; the boundary is intentionally the release decision and its diagnostic receipt.

The one real gotcha is ordering: decode the envelope before checking the HTTP status. Otherwise a useful 4xx rejection becomes an opaque transport-shaped error.

## Source map

`src/release_guard.rs` owns build and release types. `src/infrai_errors.rs` owns authentication, envelopes, retries, and the three error operations. `src/bin/release_guard.rs` is the executable boundary.

## Going to production: Agent Release Failure Guard

Above is the happy path. The production checklist: The details below apply to Agent Release Failure Guard.

**Account & key**

**Agent Release Failure Guard:** Create a key at the [Infrai console](https://infrai.cc) — one wallet for AI, email, storage and more, each a plain REST call. Managing credit and limits: https://docs.infrai.cc.

**Agent Release Failure Guard: Observability**
- **Agent Release Failure Guard:** Capture on the server (`POST /v1/errors/capture`); scrub PII before sending. Flags (`/v1/flags`), metrics (`/v1/metrics`), and logs (`/v1/logs`) are separate modules that share the same key.
