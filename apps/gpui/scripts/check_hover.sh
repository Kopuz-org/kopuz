#!/usr/bin/env bash
# An element carries one hover listener, and gpui-component spends a Button's on
# its tooltip. A second is a debug panic and a dropped listener in release, so a
# Button may not carry both: wrap it with `components::hover::framed` instead.
#
# Flags a Button chain holding `.tooltip(` with a string and `.on_hover(`, which
# is the shape that crashed the artist header and the playlist cards. A tooltip
# taking a closure is gpui's own and writes a separate field, so it is left be.
set -uo pipefail
cd "$(dirname "$0")/.."

found=$(
    for file in $(grep -rl "on_hover" . --include="*.rs" 2>/dev/null); do
        awk -v file="$file" '
            { line[NR] = $0 }
            END {
                for (i = 1; i <= NR; i++) {
                    if (line[i] !~ /\.on_hover\(/) continue
                    # Walk back to whatever opened this chain.
                    start = i
                    for (j = i; j > 1 && j > i - 40; j--) {
                        if (line[j] ~ /(Button::new|round_button|utility|mark|bare)\(/) { start = j; break }
                        if (line[j] ~ /^[[:space:]]*(div|h_flex|v_flex)\(\)/) { start = 0; break }
                    }
                    if (start == 0 || start == i) continue
                    for (j = start; j <= i + 40 && j <= NR; j++) {
                        if (line[j] ~ /\.tooltip\(/ && line[j] !~ /\.tooltip\((move )?\|/) {
                            printf "%s:%d  a Button carries both .on_hover and .tooltip\n", file, i
                            break
                        }
                    }
                }
            }
        ' "$file"
    done
)

if [ -z "$found" ]; then
    echo "No button carries both a tooltip and a hover listener."
    exit 0
fi

echo "$found" >&2
echo "Wrap the button in components::hover::framed and hover the frame instead." >&2
exit 1
