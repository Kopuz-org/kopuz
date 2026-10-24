#!/usr/bin/env bash
# The daemon owns what kind a source is; this frontend may not name a local one, as kopuz's frontends may not.
set -uo pipefail
cd "$(dirname "$0")/.."

forbidden='(?i)Source::Local|LocalLibrary|local_source|local_library|music_directory|is_local\b|SourceKind|upsert_local|LocalSourceDraft|t\("local|t\("[a-z_]+_local"|local_?sources?\b|\blocal_?librar|(?-i:\bLocal\b)'

found=$(grep -rnP --include='*.rs' "$forbidden" .)
if [ -n "$found" ]; then
    echo "$found" >&2
    echo "local-source reference(s) in the frontend: the daemon owns what a source is" >&2
    exit 1
fi
echo "no local-source concept in the frontend"
