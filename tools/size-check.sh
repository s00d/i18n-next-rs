#!/usr/bin/env bash
# Size gate (DESIGN 2.4): builds tools/size-probe for wasm32-unknown-unknown in the
# micro (default-features = false), full (default features) and guest (micro + tauri-guest)
# variants, shrinks each with `wasm-opt -Oz`, prints the bytes, and compares with a base
# checkout. The bytes
# are net: the probe minus the same probe without i18n-next (`--features empty`), so the
# constant std runtime of a wasm binary does not dilute the percentage.
#
#   tools/size-check.sh               # print sizes only (push to main)
#   tools/size-check.sh <base-dir>    # also compare with the same build of <base-dir>
#
# Fails when a variant is over its budget (tools/size-budget.env: MICRO_BUDGET, FULL_BUDGET,
# GUEST_BUDGET, net bytes). Growth over SIZE_THRESHOLD_PCT (default 2) percent against the base is reported
# as a warning to read in the PR, not a failure: a new feature legitimately adds code, and the
# owner's guideline is "about 100 KB is a real size, don't contort the design for less".
# A base without tools/size-probe has no baseline: the sizes are printed.
# Size is deterministic for one toolchain and one Cargo.lock, so this does not flake.
set -euo pipefail

THRESHOLD="${SIZE_THRESHOLD_PCT:-2}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source-path=SCRIPTDIR source=size-budget.env
. "${HERE}/tools/size-budget.env"
BASE="${1:-}"
# The full build carries CLDR number data for these locales only (DESIGN 2.4: measured for 3).
export I18N_NEXT_LOCALES="${I18N_NEXT_LOCALES:-en,ru,de}"
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
  # A probe that links `js-sys` (the browser zones of `tz`) carries wasm-bindgen's describe
  # metadata (~290 KB) that an app's build removes: run wasm-bindgen as that build does.
  if grep -q '__wasm_bindgen_unstable' "${wasm}"; then
    local bindgen
    bindgen="$(bindgen_for "${repo}")" || return 1
    "${bindgen}" --target web --out-dir "${target_dir}/bindgen-${variant}" "${wasm}" >&2 || return 1
    wasm="${target_dir}/bindgen-${variant}/size_probe_bg.wasm"
  fi
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext \
    "${wasm}" -o "${opt}" || return 1
  wc -c < "${opt}" | tr -d ' ' || return 1
}

# bindgen_for <repo-dir> -> path of a wasm-bindgen CLI of exactly the version the probe's
# Cargo.lock pins (the CLI refuses a module built by another version). Tries $WASM_BINDGEN,
# the one on PATH and target/tools/bin; else installs that version under
# target/tools/wasm-bindgen-<version> (once; it goes away with target/).
bindgen_for() {
  local want cand
  want="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' \
    "$1/tools/size-probe/Cargo.lock")"
  [[ -n "${want}" ]] || { echo "size-check: no wasm-bindgen in $1/tools/size-probe/Cargo.lock" >&2; return 1; }
  local own="${HERE}/target/tools/wasm-bindgen-${want}"
  for cand in "${WASM_BINDGEN:-}" wasm-bindgen "${HERE}/target/tools/bin/wasm-bindgen" "${own}/bin/wasm-bindgen"; do
    [[ -n "${cand}" ]] && command -v "${cand}" >/dev/null || continue
    if [[ "$("${cand}" --version 2>/dev/null)" == "wasm-bindgen ${want}" ]]; then
      command -v "${cand}"
      return 0
    fi
  done
  echo "size-check: installing wasm-bindgen-cli ${want} into ${own}" >&2
  cargo install --locked --quiet --root "${own}" wasm-bindgen-cli --version "=${want}" >&2 || return 1
  echo "${own}/bin/wasm-bindgen"
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
for variant in micro full guest; do
  head_bytes="$(net "${HERE}" "${variant}" "${head_rt}")"
  # a base from before a variant existed (the `guest` probe came with M9) has no baseline for it
  if "${has_base}" && { [[ "${variant}" == "micro" ]] || grep -q "^${variant} = " "${BASE}/tools/size-probe/Cargo.toml"; }; then
    base_bytes="$(net "${BASE}" "${variant}" "${base_rt}")"
    # growth in hundredths of a percent, integer math only (an empty base counts as 1 byte)
    growth=$(( (head_bytes - base_bytes) * 10000 / (base_bytes > 0 ? base_bytes : 1) ))
    printf '%-6s %8s bytes  (base %s, %s)\n' "${variant}" "${head_bytes}" \
      "${base_bytes}" "$(fmt_growth "${growth}")"
    if (( growth > THRESHOLD * 100 )); then
      echo "::warning::${variant} .wasm grew more than ${THRESHOLD}%: ${base_bytes} -> ${head_bytes} bytes"
    fi
  else
    printf '%-6s %8s bytes  (no baseline)\n' "${variant}" "${head_bytes}"
  fi
  budget_var="$(echo "${variant}" | tr '[:lower:]' '[:upper:]')_BUDGET"
  budget="${!budget_var}"
  if (( head_bytes > budget )); then
    echo "::error::${variant} .wasm is ${head_bytes} net bytes, over its budget of ${budget} (tools/size-budget.env)"
    status=1
  fi
done
exit "${status}"
