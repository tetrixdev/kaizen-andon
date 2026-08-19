# Kaizen Desktop

A small window that keeps one number in view: how much of today is not
accounted for yet. It gets harder to ignore as that grows, and one click hands
the day to whichever AI session you are already in.

It is the desktop half of [Kaizen](https://github.com/tetrixdev/kaizen). The
ledger works from the browser on its own; this is the ambient surface.

Internally the lamp is called the *andon*, after the 行灯 lantern on a Toyota
line that lights when something has deviated. That word stays in the code and
the repo name and never appears in anything a user reads.

## What it does

- Shows the day's unaccounted time, escalating **green → amber → red** on one
  threshold you set per context in Kaizen.
- Once the day adds up, the question changes: the glyph turns from 灯 to 印 and
  it starts asking whether the work reached whatever system you bill from.
- Opens into the day's ledger: every entry at its real position and length,
  work solid, rest hatched, holes dashed.
- Never writes anywhere but your own Kaizen, and holds no AI credentials of any
  kind.

## Design

The full specification and the visual design live in the Kaizen repo under
`design/`: `ANDON.md` for the rules and the data model, `andon.html` for every
state at 1:1.

## Connecting it

The build is generic; no instance is baked in. On first run it asks for your
Kaizen address, discovers the rest from
`/.well-known/oauth-authorization-server`, and registers itself dynamically
(RFC 7591), the same mechanism Kaizen already uses for its Claude connector.
Nothing but the returned token is stored, and that goes to the OS credential
store.

The smooth path is the **Connect** button in Kaizen's own settings, which hands
the address over via a `kaizen-andon://` link so there is nothing to type.

## Traps

**Icons must be RGBA.** `tauri::generate_context!` panics at compile time on an
RGB PNG with `icon ... is not RGBA`, and it does so only where the PNGs are
read: a Windows build validates the `.ico` and sails past it, so CI can be
green while a Linux check fails. Generate with an alpha channel.

**Check, do not build, when disk is short.** A full Tauri debug build lands at
4-6GB of target directory, larger than the 3.2GB dev image itself. `cargo
check` with `CARGO_PROFILE_DEV_DEBUG=0` brings that to about 1.5GB. `docker
builder prune` afterwards matters too: the build cache holds a full duplicate
of the image layers.

## Building

```
npm install --global @tauri-apps/cli   # or use npx
cargo tauri dev
cargo tauri build
```

Windows installers are built by CI on a tagged push (`v*`), which also
publishes `latest.json` for the self-updater.

### Signing

Updates are cryptographically signed and the app refuses any it cannot verify.
Generate the keypair once with `cargo tauri signer generate`, then set
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` as
repository secrets and put the public key in `tauri.conf.json`.

Windows *code* signing is deliberately skipped. SmartScreen fires on Mark of
the Web, which the browser applies at download time, so it is a first-install
event rather than a per-update one: the updater fetches installers itself, so
no mark is applied and updates do not re-trigger it. The installer also runs
per-user, which avoids UAC and the unknown-publisher prompt entirely.
