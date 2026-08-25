# 20. Desktop shell: Tauri v2 + React in WKWebView, one argv-dispatching binary

Date: 2026-08-25T00:00:00Z

## Status

Proposed

(Authored by `r1.s1.w1`, the first work item that actually builds the shell. It is
`Proposed` on purpose: ossify's bones protocol mints a decision `Accepted` only once a
release has exercised it, and at authoring time the shell has been built but not yet
shipped through a closed release. `r1.s1`'s close flips it to `Accepted`. This ADR
closes the half of **ADR-0003** that ADR-0015 explicitly refused ownership of, and the
half of **ADR-0019** that its *Deliberately out of scope* section named and deferred.)

## Context

PulseTrader is a CLI proof-of-concept with a ten-screen desktop design that has only
ever rendered as a browser mock. ADR-0019 pinned the exercised stack — Rust core,
embedded SQLite, Parquet — and expressly declined to decide the desktop shell, calling
it "a direction, not a settled contract". ADR-0015 wrote up the **zero-sidecar** half of
ADR-0003's compound queue entry and left the **Tauri desktop shell** half `Proposed`,
noting it "needs its own dedicated ADR when a slice builds it". This is that ADR.

Two things constrain the decision rather than being free choices:

- **ADR-0015's "one shippable artifact, zero sidecar processes."** Whatever the desktop
  shell is, it may not add a second process to supervise or a second artifact to sign.
- **`src/main.rs` and `src/lib.rs` compile as separate crates** (audit C1). The binary
  reaches only the library's `pub` surface — at baseline, exactly `pulse::run()`.
  Tauri needs a binary that starts its runtime, so the entry point is a real open
  question, not a detail.

The mock supplies most of the rest of the evidence, and it points one way:

- **Every screen already draws its own titlebar and traffic lights** and uses no native
  control anywhere. There is no macOS control in the design to lose, which is what rules
  SwiftUI out — a native shell would buy native chrome the design does not want and
  would cost a full second implementation of every screen.
- **`macos-window.jsx` is unreferenced** — an abandoned native-chrome exploration, not a
  live direction.
- **The design is React plus plain CSS custom properties with no component library**, so
  the port into a webview is mechanical rather than a rewrite.
- **The load-carrying interaction is streaming** — the compose run emits tokens and
  step events continuously — so the seam between core and UI has to be a first-class
  typed stream, not a request/response afterthought.

## Decision

**Tauri v2** hosting a **React + Vite + TypeScript** frontend in macOS **WKWebView**,
with **`tauri-specta`**-generated TypeScript bindings as the only typed seam between the
Rust core and the UI, and an **undecorated** window the application decorates itself.

**Executable topology: one binary that dispatches on argv.** The bundle contains a
single executable. `main` inspects its own arguments:

- **No arguments** — the shape of a Finder / LaunchServices launch, which attaches no
  terminal — selects **GUI** startup and hands control to the Tauri runtime.
- **Any argument** selects the **CLI** path and `pulse::run()` behaves exactly as it
  does today.

Finder's historical `-psn_<n>_<n>` process-serial-number argument and AppKit's
`-NSDocumentRevisionsDebugMode` are filtered out before the count is taken, so a launch
that carries only OS-injected arguments is still read as "no arguments". The dispatch
itself is a pure function (`pulse::launch_mode`) over an argument iterator, so both
directions are testable without opening a window.

**Why this remains one shippable artifact, and why the alternatives do not.** ADR-0015's
rule is about the *artifact and process count*, and argv dispatch keeps both at one:
one binary inside one `PulseTrader.app`, one code-signing identity, one notarization
submission, one version number, and no supervision relationship between components. The
GUI does not spawn the CLI and the CLI does not spawn the GUI — they are two entry
paths into the same address space, chosen once at startup and never again. Shipping a
separate `pulse` CLI binary alongside the app would be two artifacts to sign and version
and would make ADR-0015 literally false; embedding a CLI binary as a bundled resource the
app shells out to would re-create exactly the sidecar ADR-0015 exists to forbid. Argv
dispatch is the only option on the table that keeps the rule true as written rather than
true-in-spirit.

**The command bus is pinned here because three later work items code against it.**

- **One serializable error shape.** Every domain error that can cross the boundary
  (`DataError` first, and `ValidationErrors`, `BacktestError`, `LlmError`,
  `ComposerError`, `ExchangeError` behind it) maps to a single `BusError { code,
  message }`, where `code` is a closed enum the frontend can branch on and `message` is
  the error's `Display` rendering. A stringified `Debug` never crosses, and a panic
  never crosses.
- **Per-invocation channels, not a global event bus.** Streaming uses a Tauri v2
  `Channel<T>` handed to the command that starts the run. The channel *is* the
  correlation: a second compose run gets a second channel and cannot be mistaken for the
  first. A global event bus with correlation ids bolted on was rejected — it makes every
  subscriber responsible for filtering, and a missed filter is a cross-run data leak
  that type-checks.
- **One append-only registration point.** Commands are registered in one list and routes
  in one table, one line per screen, so two work items each adding one screen conflict
  textually and never semantically.
- **Managed state owns the expensive, shared, long-lived things** — the SQLite pool and
  the repositories built over it. Commands construct per-call only what is cheap and
  request-scoped.

**Least-privilege capabilities.** The frontend is granted core app/event/window/webview
permissions and nothing else. No filesystem, shell or HTTP capability is granted, and
that absence is asserted by a check script rather than trusted to review — an
ungranted permission is only a security property if something fails when it reappears.

**The window is pinned to 1440x900, `resizable: false`.** The mock's `installFit()` /
inline `fit()` CSS `transform: scale()` is **not** ported. That scaling exists because
the mock ran in a browser tab whose size it did not control; an application that owns
its own window does not need it, and scaling the whole canvas to fake responsiveness
would blur text and desynchronize hit targets from their painted positions.

## Consequences

**Recorded risk 1 — WKWebView is not Chromium, and the mock has only ever rendered in
Chrome.** Every visual assumption in the design was validated against Blink. WebKit
differs in font rendering and metrics, in `backdrop-filter` behaviour, in scrollbar
styling, and in the exact interpretation of some flexbox and grid edge cases. Nothing in
this work item discharges that risk, because a deliberately unstyled placeholder page
cannot reveal a rendering difference. **`r1.s1.w5` discharges it** when the real design
system first renders in the real webview, and it is the item that will pay for any
divergence found.

**Recorded risk 2 — the 1440x900 canvas is fixed, and `.layout` sits on fixed pixel
columns.** Responsive behaviour is unscoped work, not deferred work with a plan behind
it. Pinning the window at 1440x900 with `resizable: false` makes the fixed canvas
*correct* rather than *broken*, which is the honest position while the layout is
genuinely fixed — but it also means the app cannot yet use a larger display, cannot be
tiled, and will not fit a 1366x768 or 1280x800 laptop screen without the OS shrinking
it. **Admission condition:** the first evidence that the fixed canvas costs something
real — a target user on a smaller display, or a screen whose content genuinely
outgrows the column — admits responsive layout as a feature-map entry. Until then it
stays unbuilt rather than half-built.

**Other consequences.** The Rust core gains a large dependency graph (Tauri, `wry`,
`tao`, `objc2`) that the CLI does not need but now compiles anyway, so build times rise
for everyone; the licence surface grows and stays inside ADR-0011's permissive
allow-list. TypeScript becomes the project's second language, as ADR-0019 anticipated.
`ui/src/bindings.ts` is generated and committed, so a stale binding is a diffable
failure rather than a runtime surprise. And because the topology is argv-based, a
future need to launch the GUI *with* arguments (a URL scheme, a file open) requires
revisiting this ADR rather than quietly adding a flag — that is the intended revisit
trigger, alongside any requirement for a second process or a second artifact.

## Alternatives considered

**SwiftUI (or AppKit) native shell.** Rejected. The design draws its own titlebar and
traffic lights on every screen and uses no native control, so a native shell buys
nothing the design asked for while costing a complete second implementation of ten
screens in a second language — and it would put a language boundary in the middle of
the streaming path, which is the load-carrying interaction. The unreferenced
`macos-window.jsx` is the trace of this option already having been explored and dropped.

**Electron.** Rejected by ADR-0015 on its own, without needing any of the evidence
above: Electron ships a bundled Chromium and a Node process, which is a second runtime
to package, sign and patch, and it is the sidecar shape ADR-0015 forbids.

**Two binaries — a GUI executable plus a separate CLI executable.** Rejected. It is the
most obvious way to make the entry point unambiguous, and it fails the constraint that
matters: two artifacts to sign, notarize and version-match, and a support surface where
the two can drift. See the topology reconciliation above.

**A CLI binary embedded as a bundle resource, shelled out to by the GUI.** Rejected
outright — this is a sidecar process by any reading of ADR-0015, and it would reintroduce
IPC, process supervision and version skew between two halves of one product.

**A `--gui` flag instead of argv-count dispatch.** Rejected. Finder cannot pass a flag,
so the flag would have to default to GUI-when-absent, which is argv-count dispatch with
an extra way to get it wrong.

**A global Tauri event bus with correlation ids for streaming.** Rejected — see the
per-invocation channel decision above.

**Deferring the shell decision again and building against a stub.** Rejected. ADR-0019
already deferred once, and the cost has come due: three work items in this spine code
against the command bus, and each would otherwise discover its own answer.
