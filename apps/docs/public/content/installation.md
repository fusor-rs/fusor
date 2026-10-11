# Installation

Install the CLI, create an application, and make your first visible change.

## What you need {#requirements}

Install Rust 1.85 or newer with rustup, Rust’s official installer. It includes Cargo, the
package manager and build tool. You also need your platform’s native linker, usually
supplied by its developer tools.

Check that Rust and Cargo are available, then use the installer for your operating system
below. The CLI downloads as a ready-to-run binary; your applications build with Rust.

```sh title=Terminal
rustc --version
cargo --version
```

> Rust-only browser apps do not require Node or npm. Optional component JavaScript and
> island browser builds require Node. The framework’s browser tests also use Node.

- [Install Rust with rustup](https://rustup.rs/)

## Install on macOS or Linux {#install-unix}

Run this command in your terminal. The installer downloads the latest released CLI for your
operating system and processor, verifies its checksum, and installs fusor in ~/.fusor/bin.

```sh title=Terminal
curl -fsSL https://fusor.build/install.sh | sh
```

> Follow the PATH instructions printed by the installer. For the default location, add
> `export PATH="$HOME/.fusor/bin:$PATH"` to ~/.zshrc or ~/.bashrc and reload your shell. To
> use fusor immediately, run that export command in the current terminal too.

## Install on Windows {#install-windows}

Run this command in PowerShell. The installer downloads the latest released Windows CLI,
verifies its checksum, and installs fusor in your user profile’s .fusor\\bin directory.

```text title=PowerShell
irm https://fusor.build/install.ps1 | iex
```

> The installer adds its bin directory to your user PATH. Open a new PowerShell window after
> installation so the commands are available.

## Upgrade fusor {#upgrade}

Run this to replace fusor with the latest stable release:

```sh title=Terminal
fusor upgrade
```

If you're already on the latest release, nothing changes. fusor upgrades the way you
installed it: with the installer above, or with `cargo install` if you installed it that way.

Each application pins the exact fusor version it was created with. After upgrading, change
the `fusor-*` versions in each application's `Cargo.toml` to the new release. `fusor doctor`
lists any that don't match.

- [What fusor upgrade checks](/docs/cli#upgrade)

## Generate an application {#create}

In the directory where you keep your projects, create an application named `my-app`. The CLI
writes the manifest, Rust modules, HTML templates, CSS, and build script, then prepares and
checks the app. Its framework dependencies come from crates.io.

```sh title=Terminal
fusor --version
fusor new my-app
cd my-app
```

> Choose a new directory name: the CLI refuses to overwrite an existing destination. The
> first run prepares the Rust toolchain, Wasm target, and matching build tools, so it can
> take longer than later runs.

## Run it and check the result {#run}

The previous commands leave you inside the generated `my-app` directory. Project creation
has already prepared and checked the application. Start the development server here.

`fusor dev` builds the app and watches for changes.

Open `http://127.0.0.1:8090/`. You should see “Rust, inside HTML.” and two Increment
buttons. Click the first: both Shared values become 1, but only its “Clicks here” value
becomes 1.

```sh title=Terminal
# In my-app:
fusor dev --port 8090
```

> Keep this terminal running. If the port is occupied, choose another `--port` value. Use
> the matching URL printed by the command.

## Make a visible change {#first-edit}

Open `my-app/web/index.html`. Change the heading text to “My first fusor app” and save. The
browser should update. The generated Rust state is in `src/app.rs`, and each counter’s local
state is in `src/counter.rs`. Next, read Project structure to understand why one signal is
shared while each counter keeps its own click count.

```html title=web/index.html · replace the heading
<h1>My first fusor app</h1>
```

- [Walk through the generated app](/docs/project-structure)

## Check and preview a release build {#ship}

In another terminal inside `my-app`, run `fusor check`, then build the optimized release
with `fusor build`.

The generated `dist/` directory contains HTML, public assets, JavaScript glue, and Wasm. Its
HTML preloads the JavaScript modules and Wasm, so the browser fetches them together. Use
`fusor preview` to preview those files locally. To deploy, publish the contents of `dist/`
at your configured base path.

```sh title=Terminal
# In my-app:
fusor check
fusor build --locked
fusor preview --port 8092
```

> Open http://127.0.0.1:8092/ for the release preview. Project creation writes Cargo.lock;
> `--locked` prevents later builds from changing dependency resolution.

- [Configure deep-link fallbacks](/docs/routing#hosting)

## If the first run fails {#troubleshooting}

“fusor: command not found” means the installed CLI is not on your shell’s PATH. On macOS or
Linux, follow the installer’s instructions for ~/.fusor/bin. On Windows, open a new
PowerShell window after installation.

If project preparation fails, the generated sources are preserved. Run `fusor install`
inside the app to resume. A compile error in HTML usually points to the Rust expression you
wrote in the HTML; resolve the first diagnostic before adding more code.

- [Commands and diagnostics](/docs/tooling)
