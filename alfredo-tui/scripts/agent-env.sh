#!/usr/bin/env bash
# Prepare a cloud sandbox for automated Alfredo runs (daily scan / build agents).
# Idempotent and safe to re-run. SOURCE it so the cargo settings stay exported:
#
#   source alfredo-tui/scripts/agent-env.sh            # tools + disk-friendly cargo env
#   AGENT_ENV_TOOLCHAIN=1 source alfredo-tui/scripts/agent-env.sh   # also install the pinned toolchain
#
# Why it exists: every run used to rediscover and install these by hand (see
# docs/agents/automation-runbook.md).
_ae_log() { printf '[agent-env] %s\n' "$*" >&2; }
_ae_root="$(git -C "$(dirname "${BASH_SOURCE[0]:-$0}")" rev-parse --show-toplevel 2>/dev/null || pwd)"
_ae_sudo=""; if [ "$(id -u)" -ne 0 ] && command -v sudo >/dev/null 2>&1; then _ae_sudo="sudo"; fi

# 1. System packages CI has but the sandbox may not: bubblewrap (worker sandbox; ~90 tests
#    and every /go smoke need it) and GNU time (peak RSS for metric M16).
_ae_pkgs=()
command -v bwrap >/dev/null 2>&1 || _ae_pkgs+=(bubblewrap)
[ -x /usr/bin/time ] || _ae_pkgs+=(time)
if [ "${#_ae_pkgs[@]}" -gt 0 ]; then
  _ae_log "installing ${_ae_pkgs[*]}"
  { $_ae_sudo apt-get update -qq && $_ae_sudo apt-get install -y -qq "${_ae_pkgs[@]}" >/dev/null; } \
    || _ae_log "WARN: apt install failed for ${_ae_pkgs[*]}; bubblewrap-dependent tests will fail as environmental"
fi

# 2. cargo-audit at the version CI pins.
if ! cargo audit --version 2>/dev/null | grep -q ' 0\.22\.2$'; then
  _ae_log "installing cargo-audit 0.22.2"
  cargo install cargo-audit --version 0.22.2 --locked --quiet || _ae_log "WARN: cargo-audit install failed"
fi

# 3. Disk: the sandbox has a small disk allowance; debug info and incremental caches
#    filled it in earlier runs. One shared target dir across worktrees/branches.
export CARGO_PROFILE_DEV_DEBUG=0
export CARGO_INCREMENTAL=0
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$_ae_root/alfredo-tui/target}"

# 4. Toolchain: CI pins RUSTUP_TOOLCHAIN (see .github/workflows/rust-terminal.yml); the
#    release packager refuses other versions. Installing it costs disk, so it is opt-in.
_ae_pin="$(sed -n 's/^  RUSTUP_TOOLCHAIN: *//p' "$_ae_root/.github/workflows/rust-terminal.yml" | head -1)"
if [ -n "$_ae_pin" ]; then
  if [ "${AGENT_ENV_TOOLCHAIN:-0}" = "1" ]; then
    rustup toolchain install "$_ae_pin" --profile minimal -c clippy -c rustfmt >/dev/null 2>&1 \
      && export RUSTUP_TOOLCHAIN="$_ae_pin" && _ae_log "using pinned toolchain $_ae_pin" \
      || _ae_log "WARN: could not install toolchain $_ae_pin"
  else
    _ae_log "CI pins Rust $_ae_pin; sandbox has $(rustc --version 2>/dev/null | cut -d' ' -f2) (set AGENT_ENV_TOOLCHAIN=1 to match; only release_smoke needs it)"
  fi
fi

# 5. Summary for the run log.
_ae_log "bwrap: $(command -v bwrap >/dev/null && echo yes || echo NO) · time: $([ -x /usr/bin/time ] && echo yes || echo NO) · cargo-audit: $(cargo audit --version 2>/dev/null | awk '{print $2}' || echo NO)"
_ae_log "user: $(id -un) (CI runs as non-root) · cpus: $(nproc) · free disk: $(df -h "$_ae_root" | awk 'NR==2{print $4}') · target: $CARGO_TARGET_DIR"
unset _ae_pkgs _ae_sudo _ae_pin _ae_root
