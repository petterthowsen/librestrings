#!/usr/bin/env bash
# Download measured bowed-string reference data into data/ (gitignored).
# Sources and licenses: docs/Violin Reference Recordings.md, sections 1 and 3.
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
# guettler-*, schelleng-*: mdw Vienna (Lampis, Chatziioannou, Mayer), CC BY 4.0.
# iowa-*: University of Iowa MIS, four bowed instruments recorded the same way
#   (mono, 16-bit 44.1 kHz), "may be downloaded and used for any projects,
#   without restrictions". Converted from AIFF to WAV, which needs ffmpeg.
#   The viola set's 16/44.1 files hold 96 kHz audio (A4 comes out at 202 Hz):
#   compare it with `compare --instrument viola --file-rate 96000`.
MANIFEST="
iowa-violin|Violin.arco.mono.1644.1.zip|57f81bd1b50480f0f5ff2963dc88fad8|https://theremin.music.uiowa.edu/sound%20files/MIS/Strings/violin2012/Violin.arco.mono.1644.1.zip
iowa-viola|Viola.arco.mono.1644.1.zip|25ca294beda345c0514fc9d44f6a6f22|https://theremin.music.uiowa.edu/sound%20files/MIS/Strings/viola2012/Viola.arco.mono.1644.1.zip
iowa-cello|Cello.arco.mono.1644.1.zip|81f12cc1df9dd052a1dd2f1d6611fa30|https://theremin.music.uiowa.edu/sound%20files/MIS/Strings/cello2012/Cello.arco.mono.1644.1.zip
iowa-bass|Bass.arco.mono.1644.1.zip|54ac28589dfbe69b6233f81c6ce6db53|https://theremin.music.uiowa.edu/sound%20files/MIS/Strings/doublebass2012/Bass.arco.mono.1644.1.zip
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
        *.zip) unzip -q -o "$dest/$file" -d "$dest" -x '__MACOSX/*' ;;
        *.7z)
            7z x -y -bd -o"$dest" "$dest/$file" >/dev/null
            rm "$dest/$file"
            python3 scripts/compact-schelleng.py "$dest"
            ;;
    esac
    rm -f "$dest/$file"
    for aif in "$dest"/*.aif; do
        [ -e "$aif" ] || continue
        ffmpeg -loglevel error -y -i "$aif" "${aif%.aif}.wav"
        rm "$aif"
    done
    touch "$dest/.extracted"
done
