# NAGX

NAGX is a native, SSH-native visual operations environment for Windows clients connecting to Linux servers.

## Principles
- Real PTY terminal: bash/zsh/vim/ssh/docker/systemctl/tmux and interactive CLIs work unchanged.
- Native NAGX Connect client; no browser dependency for the desktop client.
- Structured NXP protocol for telemetry, control, sessions and future DevOps providers.
- Shared server collectors instead of per-window polling.
- Explicit permissions for write/execute/admin operations.
- Low-resource rendering with adaptive refresh and no decorative heavy effects.

## Repository layout
- `clients/nagx-connect` — Windows-first native client shell.
- `apps/nagd` — Linux server daemon.
- `crates/nagx-terminal` — terminal state and PTY-facing abstractions.
- `crates/nagx-protocol` — NXP wire model.
- `crates/nagx-model` — shared resource/session state.
- `crates/nagx-core` — runtime primitives.
- `crates/nagx-session` — reconnectable session model.
- `crates/nagx-config` — configuration model.

This first commit establishes the production-oriented workspace foundation. UI, real PTY transport, SSH channel integration, telemetry providers and installers are layered on top without changing the native terminal contract.
