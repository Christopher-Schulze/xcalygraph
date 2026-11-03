set -eu
cleanup() { :; }
on_verbose() { :; }
on_output() { :; }
on_invalid() { :; }
on_fallback() { :; }
on_missing() { :; }
trap cleanup EXIT
trap 'cleanup' INT
trap "cleanup" TERM
trap 'cleanup; on_verbose' HUP
trap - EXIT
trap -p EXIT
while getopts ':ab:cd' option; do
  case "$option" in
    a) on_verbose ;;
    b) on_output "$OPTARG" ;;
    c) on_verbose ;;
    \?) on_invalid ;;
    :) on_missing ;;
    *) on_fallback ;;
  esac
done
while getopts 'z' selected; do
  case "$other" in
    z) on_invalid ;;
  esac
done
while getopts "$DYNAMIC_OPTIONS" dynamic; do
  case "$dynamic" in
    q) on_output ;;
  esac
done
while getopts 'xy' ordered; do
  case "$ordered" in
    *) on_fallback ;;
    x) on_invalid ;;
  esac
done
while getopts 'mn' patterned; do
  case "$patterned" in
    [mn]) on_fallback ;;
    m) on_invalid ;;
    n) on_output ;;
  esac
done
