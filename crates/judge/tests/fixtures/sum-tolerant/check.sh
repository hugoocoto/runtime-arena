#!/bin/sh
# testlib-style checker: check INPUT OUTPUT ANSWER. Accepts answers off by at most 1.
read -r out < "$2" || { echo "empty output" >&2; exit 1; }
read -r ans < "$3"
diff=$((out - ans))
if [ "$diff" -ge -1 ] && [ "$diff" -le 1 ]; then exit 0; fi
echo "expected $ans (+-1), found $out" >&2
exit 1
