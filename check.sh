#!/bin/bash
# Local stand-in for the CI workflow. Seconds instead of seven minutes.
#
# TWO THINGS KEEP THIS OFF THE DISK CLIFF, and both matter:
#
#   cargo check, not cargo build. Type-checking skips codegen and linking
#   entirely, which is where most of a target directory comes from. CI still
#   does the real build; locally we only need to know it compiles.
#
#   debug=0. Debug info is the bulk of what remains. A full Tauri debug build
#   lands at 4-6GB of target directory, which is larger than the 3.2GB image
#   it builds in.
#
# There is also a floor: if free space drops under FLOOR_GB the run stops
# rather than taking the machine to 100%, which has happened twice.
set -uo pipefail

FLOOR_GB=2
cd "$(dirname "$0")"

free_gb() { df --output=avail -BG / | tail -1 | tr -dc '0-9'; }

if [[ $(free_gb) -lt $((FLOOR_GB + 1)) ]]; then
    echo "only $(free_gb)GB free, need at least $((FLOOR_GB + 1)). Not starting."
    exit 1
fi

docker build -q -f Dockerfile.dev -t kaizen-andon-dev . >/dev/null || exit 1

# Watchdog: kills the run if the disk gets tight.
( while sleep 10; do
      [[ $(free_gb) -lt $FLOOR_GB ]] && {
          echo "!! under ${FLOOR_GB}GB, stopping" >&2
          docker ps -q --filter "ancestor=kaizen-andon-dev" | xargs -r docker kill >/dev/null 2>&1
          exit 1
      }
  done ) &
watchdog=$!
trap 'kill $watchdog 2>/dev/null' EXIT

run() {
    docker run --rm \
        -e CARGO_PROFILE_DEV_DEBUG=0 \
        -e CARGO_PROFILE_TEST_DEBUG=0 \
        -e CARGO_INCREMENTAL=0 \
        -v "$PWD":/app \
        -v kaizen-andon-target:/app/src-tauri/target \
        -v kaizen-andon-registry:/usr/local/cargo/registry \
        -w /app/src-tauri kaizen-andon-dev "$@"
}

fail=0
for check in "check:cargo check --all-targets" \
             "tests:cargo test" \
             "clippy:cargo clippy --all-targets -- -D warnings" \
             "fmt:cargo fmt --check"; do
    name="${check%%:*}"
    printf '\n=== %s ===\n' "$name"
    # shellcheck disable=SC2086
    run ${check#*:} || fail=1
done

printf '\n%sGB free after\n' "$(free_gb)"
[[ $fail -eq 0 ]] && echo "all four green" || echo "something failed above"
exit $fail
