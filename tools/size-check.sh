#!/usr/bin/env bash
# Size gate (DESIGN 2.4): builds tools/size-probe for wasm32-unknown-unknown in the
# micro (default-features = false) and full (default features) variants, shrinks each
# with `wasm-opt -Oz`, prints the bytes, and compares with a base checkout. The bytes
# are net: the probe minus the same probe without i18n-next (`--features empty`), so the
# constant std runtime of a wasm binary does not dilute the percentage.
#
#   tools/size-check.sh               # print sizes only (push to main)
#   tools/size-check.sh <base-dir>    # also compare with the same build of <base-dir>
#
# Fails when a variant grows by more than SIZE_THRESHOLD_PCT (default 2) percent.
# A base without tools/size-probe has no baseline: the sizes are printed, nothing fails.
# Size is deterministic for one toolchain and one Cargo.lock, so this does not flake.
set -euo pipefail

THRESHOLD="${SIZE_THRESHOLD_PCT:-2}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASE="${1:-}"
OUT="${HERE}/target/size"
mkdir -p "${OUT}"

# measure <repo-dir> <variant> -> prints bytes after wasm-opt -Oz
# Runs inside `$(...)`, where bash drops `set -e`: every step returns on failure
# explicitly, so a broken build fails the script instead of reporting a stale size.
measure() {
  local repo="$1" variant="$2" features=()
  [[ "${variant}" != "micro" ]] && features=(--features "${variant}")
  local target_dir="${OUT}/target-$(basename "${repo}")"
  cargo build --release --locked --manifest-path "${repo}/tools/size-probe/Cargo.toml" \
    --target wasm32-unknown-unknown --target-dir "${target_dir}" "${features[@]}" >&2 || return 1
  local wasm="${target_dir}/wasm32-unknown-unknown/release/size_probe.wasm"
  local opt="${OUT}/$(basename "${repo}")-${variant}.wasm"
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    "${wasm}" -o "${opt}" || return 1
  wc -c < "${opt}" | tr -d ' ' || return 1
}

# fmt_growth <hundredths-of-a-percent> -> "+1.05%" / "-0.50%"
fmt_growth() {
  local g="$1" sign="+"
  (( g < 0 )) && { sign="-"; g=$(( -g )); }
  printf '%s%d.%02d%%' "${sign}" "$(( g / 100 ))" "$(( g % 100 ))"
}

# runtime <repo-dir> -> bytes of the probe without i18n-next (the std runtime of a wasm
# binary). 0 for an older probe without the `empty` feature: that one was a cdylib
# with no runtime to subtract.
runtime() {
  if grep -q '^empty = ' "$1/tools/size-probe/Cargo.toml"; then
    measure "$1" empty || return 1
  else
    echo 0
  fi
}

# net <repo-dir> <variant> <runtime-bytes> -> bytes i18n-next adds to the probe
net() {
  local raw
  raw="$(measure "$1" "$2")" || return 1
  echo $(( raw - $3 ))
}

has_base=false
[[ -n "${BASE}" && -f "${BASE}/tools/size-probe/Cargo.toml" ]] && has_base=true
head_rt="$(runtime "${HERE}")"
"${has_base}" && base_rt="$(runtime "${BASE}")"

status=0
for variant in micro full; do
  head_bytes="$(net "${HERE}" "${variant}" "${head_rt}")"
  if "${has_base}"; then
    base_bytes="$(net "${BASE}" "${variant}" "${base_rt}")"
    # growth in hundredths of a percent, integer math only (an empty base counts as 1 byte)
    growth=$(( (head_bytes - base_bytes) * 10000 / (base_bytes > 0 ? base_bytes : 1) ))
    printf '%-6s %8s bytes  (base %s, %s)\n' "${variant}" "${head_bytes}" \
      "${base_bytes}" "$(fmt_growth "${growth}")"
    if (( growth > THRESHOLD * 100 )); then
      echo "::error::${variant} .wasm grew more than ${THRESHOLD}%: ${base_bytes} -> ${head_bytes} bytes"
      status=1
    fi
  else
    printf '%-6s %8s bytes  (no baseline)\n' "${variant}" "${head_bytes}"
  fi
done
exit "${status}"
