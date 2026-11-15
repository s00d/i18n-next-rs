#!/usr/bin/env bash
# Size gate (DESIGN 2.4): builds tools/size-probe for wasm32-unknown-unknown in the
# micro (default-features = false) and full (default features) variants, shrinks each
# with `wasm-opt -Oz`, prints the bytes, and compares with a base checkout.
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
measure() {
  local repo="$1" variant="$2" features=()
  [[ "${variant}" == "full" ]] && features=(--features full)
  local target_dir="${OUT}/target-$(basename "${repo}")"
  cargo build --release --locked --manifest-path "${repo}/tools/size-probe/Cargo.toml" \
    --target wasm32-unknown-unknown --target-dir "${target_dir}" "${features[@]}" >&2
  local wasm="${target_dir}/wasm32-unknown-unknown/release/size_probe.wasm"
  local opt="${OUT}/$(basename "${repo}")-${variant}.wasm"
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    "${wasm}" -o "${opt}"
  wc -c < "${opt}" | tr -d ' '
}

status=0
for variant in micro full; do
  head_bytes="$(measure "${HERE}" "${variant}")"
  if [[ -n "${BASE}" && -f "${BASE}/tools/size-probe/Cargo.toml" ]]; then
    base_bytes="$(measure "${BASE}" "${variant}")"
    # growth in hundredths of a percent, integer math only
    growth=$(( (head_bytes - base_bytes) * 10000 / base_bytes ))
    printf '%-6s %8s bytes  (base %s, %+d.%02d%%)\n' "${variant}" "${head_bytes}" \
      "${base_bytes}" "$(( growth / 100 ))" "$(( (growth < 0 ? -growth : growth) % 100 ))"
    if (( growth > THRESHOLD * 100 )); then
      echo "::error::${variant} .wasm grew more than ${THRESHOLD}%: ${base_bytes} -> ${head_bytes} bytes"
      status=1
    fi
  else
    printf '%-6s %8s bytes  (no baseline)\n' "${variant}" "${head_bytes}"
  fi
done
exit "${status}"
