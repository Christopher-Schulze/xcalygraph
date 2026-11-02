#!/usr/bin/env bash
# Source this file to persist benchmark stdout through a waitable tee child.

OUTPUT_WRITER_FIFO=""
OUTPUT_WRITER_PID=""

start_output_writer() {
  local output_path
  (( $# == 1 )) || { echo "output-writer.sh: expected one output path" >&2; return 2; }
  output_path="$1"
  [[ -n "${output_path}" ]] || { echo "output-writer.sh: output path is empty" >&2; return 2; }
  [[ -z "${OUTPUT_WRITER_PID}" && -z "${OUTPUT_WRITER_FIFO}" ]] || {
    echo "output-writer.sh: a writer is already active" >&2
    return 1
  }

  OUTPUT_WRITER_FIFO="${output_path}.fifo"
  if [[ -e "${output_path}" || -L "${output_path}" || -e "${OUTPUT_WRITER_FIFO}" || -L "${OUTPUT_WRITER_FIFO}" ]]; then
    echo "output-writer.sh: refusing to overwrite existing output" >&2
    OUTPUT_WRITER_FIFO=""
    return 1
  fi
  if ! ( set -o noclobber; : > "${output_path}" ); then
    echo "output-writer.sh: could not create output file" >&2
    OUTPUT_WRITER_FIFO=""
    return 1
  fi
  if ! mkfifo "${OUTPUT_WRITER_FIFO}"; then
    echo "output-writer.sh: could not create output FIFO" >&2
    return 1
  fi

  exec 3>&1 || return $?
  tee -a "${output_path}" < "${OUTPUT_WRITER_FIFO}" >&3 &
  OUTPUT_WRITER_PID=$!
  if exec > "${OUTPUT_WRITER_FIFO}"; then
    return 0
  else
    local setup_status=$?
    exec >&3 3>&-
    wait "${OUTPUT_WRITER_PID}" || :
    rm -f "${OUTPUT_WRITER_FIFO}" || :
    OUTPUT_WRITER_PID=""
    OUTPUT_WRITER_FIFO=""
    return "${setup_status}"
  fi
}

finish_output_writer() {
  local writer_status=0 cleanup_status=0
  [[ -n "${OUTPUT_WRITER_PID}" && -n "${OUTPUT_WRITER_FIFO}" ]] || {
    echo "output-writer.sh: no writer is active" >&2
    return 1
  }

  exec >&3 3>&-
  wait "${OUTPUT_WRITER_PID}" || writer_status=$?
  rm -f "${OUTPUT_WRITER_FIFO}" || cleanup_status=$?
  OUTPUT_WRITER_PID=""
  OUTPUT_WRITER_FIFO=""
  (( writer_status == 0 )) || return "${writer_status}"
  return "${cleanup_status}"
}
