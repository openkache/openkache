#!/usr/bin/env bash
set -euo pipefail

if [[ "$#" -ne 4 ]]; then
  echo "usage: smoke-test.sh <server> <cli> <address> <working-directory>" >&2
  exit 2
fi

server="$1"
cli="$2"
address="$3"
working_directory="$4"

for binary in "${server}" "${cli}"; do
  if [[ ! -x "${binary}" ]]; then
    echo "missing executable: ${binary}" >&2
    exit 2
  fi
done
mkdir -p "${working_directory}"

cleanup() {
  if [[ -n "${server_pid:-}" ]]; then
    kill "${server_pid}" >/dev/null 2>&1 || true
    wait "${server_pid}" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

(
  cd "${working_directory}"
  exec "${server}" "${address}" 0 1
) >"${working_directory}/server.log" 2>&1 &
server_pid=$!

ready=false
for _ in $(seq 1 100); do
  if "${cli}" --address "${address}" ping \
    >"${working_directory}/ping.out" \
    2>"${working_directory}/ping.err"; then
    ready=true
    break
  fi
  if ! kill -0 "${server_pid}" 2>/dev/null; then
    break
  fi
  sleep 0.1
done

if [[ "${ready}" != true ]]; then
  cat "${working_directory}/server.log" >&2
  cat "${working_directory}/ping.err" >&2
  exit 1
fi

grep -Fx PONG "${working_directory}/ping.out"
"${cli}" --address "${address}" set packaging-smoke works \
  | grep -Ex 'CREATED|REPLACED'
[[ "$("${cli}" --address "${address}" get packaging-smoke)" == works ]]
"${cli}" --address "${address}" delete packaging-smoke | grep -Fx DELETED
