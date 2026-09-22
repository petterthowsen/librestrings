#!/usr/bin/env bash
# Download measured bowed-string reference data into data/ (gitignored).
# Sources and licenses: docs/Violin Reference Recordings.md, section 3.
#
# Usage: scripts/fetch-reference-data.sh [name ...]   (default: all entries below)
# Downloads resume if interrupted and are checked against Zenodo's md5.
# Schelleng archives (.7z) extract to about 70 GB of CSV and are then compacted
# to about 5 GB of FLAC by compact-schelleng.py, so they need ~75 GB free.
set -euo pipefail

cd "$(dirname "$0")/.."
DATA=data/reference
mkdir -p "$DATA"

# name | file | md5 | url
# All: mdw Vienna (Lampis, Chatziioannou, Mayer), CC BY 4.0.
MANIFEST="
guettler-waveforms|waveforms.zip|ccdf7decfa7061f62109292a850bed0c|https://zenodo.org/api/records/13374477/files/waveforms.zip/content
schelleng-typeA-s1-T1|2024-03-25_TypeA_sample1.7z|2709034cc10344f37f3488376ae3ebcd|https://zenodo.org/api/records/17749111/files/2024-03-25_TypeA_sample1.7z/content
"

WANTED=("$@")
want() {
    [ ${#WANTED[@]} -eq 0 ] && return 0
    for w in "${WANTED[@]}"; do [ "$w" = "$1" ] && return 0; done
    return 1
}

echo "$MANIFEST" | while IFS='|' read -r name file md5 url; do
    [ -z "$name" ] && continue
    want "$name" || continue
    dest="$DATA/$name"
    mkdir -p "$dest"
    if [ -f "$dest/.extracted" ]; then
        echo "$name: already extracted"
        continue
    fi
    echo "$name: downloading $file"
    curl -L --fail --retry 5 -C - -o "$dest/$file" "$url"
    echo "$name: checking md5"
    echo "$md5  $dest/$file" | md5sum -c -
    echo "$name: extracting"
    case "$file" in
        *.zip) unzip -q -o "$dest/$file" -d "$dest" ;;
        *.7z)
            7z x -y -bd -o"$dest" "$dest/$file" >/dev/null
            rm "$dest/$file"
            python3 scripts/compact-schelleng.py "$dest"
            ;;
    esac
    rm -f "$dest/$file"
    touch "$dest/.extracted"
done
