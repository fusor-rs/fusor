# Background work

Run `just dev-app fusor-workers`. This ordinary fusor app adds `fusor-worker`
and one annotated function to its existing source. Its unchanged build script
and normal CLI command package the worker automatically.

The resource returns the generated job directly, retaining its cancellation
registration. Changing the input suppresses stale results; the UI counter stays
independent of the CPU loop. Ordinary cancellation is cooperative: a busy worker
processes cancellation messages when it yields, and never rolls back mutations.

See the [worker guide](../../apps/docs/public/content/workers.md) for services, streams,
pools, shared leases and hosting requirements. The independent
[browser consumer](../../tests/fixtures/worker/src/lib.rs) exercises those APIs;
`just test worker` and `just test worker-pool` verify it in real browsers.
