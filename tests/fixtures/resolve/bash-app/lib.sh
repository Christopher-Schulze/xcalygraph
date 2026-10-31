#!/bin/sh
# A case pattern the grammar cannot parse must not hide the functions after it.

tool_parse() {
  case "${1}" in
    --*' '*) echo "spaced" ;;
    *) echo "plain" ;;
  esac
}

tool_version() {
  tool_parse "$1" && tool_echo "v1"
}

tool_echo() {
  printf '%s\n' "$1"
}
