#!/usr/bin/env bash
# Master the raw capture into web deliverables + QA artifacts
# (devrel-video step 6: master, ffprobe gates, contact sheet, poster).
set -euo pipefail
cd "$(dirname "$0")"
RAW=raw/capture.webm
ASSETS=../assets
[ -f "$RAW" ] || { echo "run capture.mjs first"; exit 1; }
# Start 1 s before the first beat (skips page load / blank paint).
START=$(python3 -c "import json;print(max(0,json.load(open('raw/beats.json'))[0]['t']-1.0))")
ffmpeg -v error -y -ss "$START" -i "$RAW" -vf "fps=30,scale=1440:-2:flags=lanczos,format=yuv420p" \
  -c:v libx264 -preset slow -crf 24 -profile:v high -movflags +faststart -an "$ASSETS/demo.mp4"
ffmpeg -v error -y -ss "$START" -i "$RAW" -vf "fps=30,scale=1440:-2:flags=lanczos" \
  -c:v libvpx-vp9 -b:v 0 -crf 36 -row-mt 1 -an "$ASSETS/demo.webm"
# Poster = the "downloads" beat (tiles + speed chart), shown before play / for reduced motion.
POSTER_T=$(python3 -c "import json;b=json.load(open('raw/beats.json'));print(max(0,[x['t'] for x in b if x['label'].startswith('Downloads')][0]+4.5-$START))")
ffmpeg -v error -y -ss "$POSTER_T" -i "$ASSETS/demo.mp4" -frames:v 1 -q:v 3 "$ASSETS/demo-poster.jpg"
# Chapters for the site (times relative to the trimmed video).
python3 - "$START" <<'PY'
import json, sys
start = float(sys.argv[1])
b = json.load(open("raw/beats.json"))
ch = [{"label": x["label"], "t": round(max(0, x["t"] - start), 2)} for x in b if x["label"] != "end"]
json.dump(ch, open("../assets/demo-chapters.json", "w"), indent=2)
print("chapters", ch)
PY
# QA gates
for f in "$ASSETS/demo.mp4" "$ASSETS/demo.webm"; do
  ffprobe -v error -select_streams v:0 -show_entries stream=width,height,r_frame_rate -show_entries format=duration,size -of compact "$f"
done
ffmpeg -v error -y -i "$ASSETS/demo.mp4" -vf "fps=1/3,scale=360:-1,tile=4x3" -frames:v 1 raw/contact-sheet.jpg
echo "contact sheet: $(pwd)/raw/contact-sheet.jpg"
