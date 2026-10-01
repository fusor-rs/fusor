# Run `just` to see every recipe.
#
# Recipes call `cargo fusor`, the alias in .cargo/config.toml that runs the CLI
# from this checkout, so they always use your local changes.

set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

npm_cache := "target/npm-cache"

[private]
default:
    @just --list

# Install everything needed for framework tests and the full example apps.
[group('setup')]
setup: setup-browser setup-examples

# Framework suites build offline, including their independent Cargo consumers.
[group('setup')]
setup-browser:
    cargo fusor install -p fusor-playground
    cargo fetch --locked --manifest-path tests/fixtures/component-tags/Cargo.toml
    cargo fetch --locked --manifest-path tests/fixtures/native-javascript/Cargo.toml
    cargo fetch --locked --manifest-path tests/fixtures/external-backend/Cargo.toml
    cargo fusor install --manifest-path tests/fixtures/worker/Cargo.toml --features pool
    npm ci
    npm ci --prefix examples/npm --cache {{npm_cache}} --ignore-scripts
    npx playwright install chromium firefox webkit

# Additional dependencies for the optional example and docs walkthroughs.
[group('setup')]
setup-examples:
    cargo fetch --locked --manifest-path examples/npm/Cargo.toml
    cargo fetch --locked --manifest-path apps/docs/tutorial/Cargo.toml
    npm ci --prefix examples/integrations --cache {{npm_cache}} --ignore-scripts
    npm ci --prefix apps/docs --cache {{npm_cache}} --ignore-scripts
    cargo fusor install -p fusor-docs

# The CLI runs from its own target directory because `cargo test` relinks
# target/debug/fusor, which Windows refuses while that binary is running.
# Formatting, workspace tests, Clippy, consumer builds and rustdoc. No browser.
[group('develop')]
check:
    cargo run --locked -p fusor-cli --bin fusor --target-dir target/cli -- repo check

# Update the lockfiles of the Cargo projects outside the workspace, which
# `setup` and `check` read with --locked. Run it after changing a workspace
# crate's dependencies and commit the result. Only required changes are made.
[group('develop')]
lock:
    cargo fetch --manifest-path tests/fixtures/consumer/Cargo.toml
    cargo fetch --manifest-path tests/fixtures/application/Cargo.toml
    cargo fetch --manifest-path tests/fixtures/component-tags/Cargo.toml
    cargo fetch --manifest-path tests/fixtures/native-javascript/Cargo.toml
    cargo fetch --manifest-path tests/fixtures/external-backend/Cargo.toml
    cargo fetch --manifest-path tests/fixtures/worker/Cargo.toml
    cargo fetch --manifest-path examples/npm/Cargo.toml
    cargo fetch --manifest-path apps/docs/tutorial/Cargo.toml

# Develop the whole site (landing page, docs, benchmarks) with live reload.
[group('develop')]
dev:
    cargo fusor dev --site

# Develop one application on its own, e.g. `just dev-app fusor-playground`.
[group('develop')]
dev-app app:
    cargo fusor dev -p {{app}}

# Build the landing page, docs and benchmarks into one site in dist/.
[group('site')]
site:
    cargo fusor build --site

# Serve the site built by `just site`.
[group('site')]
preview port="8080":
    cargo fusor preview dist --port {{port}}

# Needs `vercel login` and `vercel link --scope pirela --project fusor`, or the
# Deploy site workflow. `just deploy preview` makes a preview deployment. The
# build manifests are left out because they hold absolute paths from this machine.
# Build the site and deploy it to fusor.build.
[group('site')]
[unix]
deploy environment="production":
    @case "{{environment}}" in production|preview) ;; *) echo "deploy: expected production or preview, got {{environment}}" >&2; exit 2 ;; esac
    vercel build {{ if environment == "production" { "--prod" } else { "" } }} --yes
    find .vercel/output/static -name '.fusor-*.json' -delete
    vercel deploy --prebuilt {{ if environment == "production" { "--prod" } else { "" } }} --yes

# Run one suite from tests/tooling, e.g. `just test dev` or `just test islands`.
# On Linux, run the WebGL suites with `xvfb-run -a just test docs` (or docs-libraries).
[group('test')]
test suite:
    node tests/tooling/{{suite}}.mjs

# The Playwright suite for the playground. Needs `cargo fusor build -p fusor-playground`.
[group('test')]
test-browser *args:
    npx playwright test {{args}}

# The landing page against the assembled site. Needs `just site` or `just fixtures`.
[group('test')]
test-landing:
    node apps/landing/tests/browser.mjs

# Unit tests for the Node tooling. No browser.
[group('test')]
test-tools:
    node --test scripts/build.test.mjs
    node --test "benchmarks/tools/tests/*.test.mjs"

# Prebuild the examples and sites that some suites serve.
[group('test')]
fixtures:
    node tests/tooling/build-fixtures.mjs coherent islands site benchmark-runtime

# Required framework contracts, using focused fixtures and public-API consumers.
[group('test')]
ci-browser:
    cargo test --locked -p fusor-cli --test lifecycle --test capabilities -- --ignored
    cargo fusor build -p fusor-playground
    just test-tools
    just test javascript-build
    just test javascript-dev
    just test native-javascript
    just test worker
    just test worker-pool
    just test worker-dev
    just test consumer
    just test external-backend
    just test authoring
    just test async-components
    just test children
    just test control-flow
    just test foreach
    just test component-tags
    just test-browser
    just test template-cache
    just test dev
    just test standalone
    just test router
    just test navigation
    just test composition
    just test editor
    node tests/tooling/build-fixtures.mjs coherent islands benchmark-runtime
    just test benchmark-runtime
    just test template-resolution
    just test direct-text
    just test component-hydration
    just test bundle-bindings
    just test coherent
    just test islands
    just test delivery
    just test islands-consumer

# Full application and third-party library walkthroughs. On Linux these need Xvfb.
# Run separately from the framework gate: just setup && xvfb-run -a just ci-examples.
[group('test')]
ci-examples:
    just bench verify
    just test javascript
    just test integrations
    node tests/tooling/build-fixtures.mjs site
    just test docs
    just test docs-libraries
    just test docs-examples
    just test benchmarks
    just test-landing

# Four CPUs like the CI runner, and the latest stable Rust like CI. Share the
# host's IPC memory as recommended for Playwright's Docker image instead of
# Docker's default 64 MiB mount. Pass a command to run less,
# e.g. `just ci-docker "just test router"`.
# Run CI's browser job in Linux, on a clean copy of this checkout.
[group('test')]
[unix]
ci-docker command="just setup-browser && just ci-browser":
    docker build --pull -t fusor-ci - < tests/ci.Dockerfile
    git ls-files -coz --exclude-standard | tar --no-mac-metadata --no-xattrs --null -T - -c | docker run --rm --init -i --cpus 4 --ipc=host -e CI=true -e CARGO_INCREMENTAL=0 \
        -v fusor-ci-cargo:/usr/local/cargo/registry -v fusor-ci-target:/work/target \
        fusor-ci xvfb-run --auto-servernum bash -c 'tar -x && {{command}}'

# Build the seven benchmark workloads. Set FUSOR_WASM_OPT to use Binaryen.
[group('benchmarks')]
bench-build:
    node benchmarks/build-all.mjs

# Record a full benchmark run. It is not published until `just bench publish --id ID`.
[group('benchmarks')]
bench-run id title="":
    node benchmarks/tools/cli.mjs run --id {{id}} --title "{{title}}"

# The benchmark record tool: `just bench verify`, `just bench publish --id ID`.
[group('benchmarks')]
bench *args:
    node benchmarks/tools/cli.mjs {{args}}
