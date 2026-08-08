#!/usr/bin/env bash
set -euo pipefail

# Build the public VisualLLM workflow demo from the latest desktop captures.
# The screenshots live outside the repository; this keeps capture material out
# of git while making the finished media reproducible on the capture machine.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CAPTURE_DIR="${CAPTURE_DIR:-/home/shane/Pictures/Screenshots}"
OUT_DIR="${OUT_DIR:-$ROOT/docs/demo}"
MP4="$OUT_DIR/visualllm-demo.mp4"
GIF="$OUT_DIR/visualllm-demo.gif"
THUMB="$OUT_DIR/visualllm-demo-thumb.jpg"
FONT="/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"
BOLD="/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"

[[ -d "$CAPTURE_DIR" ]] || { echo "Capture directory not found: $CAPTURE_DIR" >&2; exit 1; }
[[ -f "$FONT" && -f "$BOLD" ]] || { echo "Required DejaVu Sans fonts are not installed." >&2; exit 1; }
mkdir -p "$OUT_DIR"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

shot() { printf '%s/%s' "$CAPTURE_DIR" "Screenshot From 2026-08-08 16-$1.png"; }

# label | source | duration | framing
SCENES=(
  'Connect a provider|31-44|4.5|full'
  'Search the model catalog|32-57|4.5|full'
  'Compare models by capability, context, price, and speed|32-21|3.5|detail'
  'Create an endpoint for a real workload|33-56|4.5|full'
  'Drag models from the vault into the endpoint|37-39|4.5|full'
  'The rightmost model answers first; the others are fallback|37-54|4.5|full'
  'Configure endpoint and editor integrations in one place|38-17|4.5|full'
)

scene_filter() {
  local label="$1" framing="$2"
  local safe_label
  safe_label="${label//\\/\\\\}"
  safe_label="${safe_label//:/\\:}"
  safe_label="${safe_label//\'/\\\'}"
  if [[ "$framing" == detail ]]; then
    # Detail captures are intentionally enlarged and letterboxed for reading.
    printf "scale=1440:-2:flags=lanczos,crop=1440:900:(iw-1440)/2:(ih-900)/2,"
  else
    printf "scale=1440:900:force_original_aspect_ratio=decrease:flags=lanczos,pad=1440:900:(ow-iw)/2:(oh-ih)/2:color=080a12,"
  fi
  printf "drawbox=x=0:y=0:w=1440:h=126:color=080a12@0.92:t=fill,"
  printf "drawtext=fontfile=%s:text='%s':fontcolor=ffffff:fontsize=30:x=54:y=31:line_spacing=8," "$BOLD" "$safe_label"
  printf "drawtext=fontfile=%s:text='VISUALLLM  /  VISUAL FALLBACK ROUTING':fontcolor=ff756d:fontsize=15:x=56:y=86" "$FONT"
}

make_scene() {
  local i="$1" label="$2" stamp="$3" duration="$4" framing="$5"
  local source
  source="$(shot "$stamp")"
  [[ -f "$source" ]] || { echo "Missing screenshot: $source" >&2; exit 1; }
  ffmpeg -hide_banner -loglevel error -y -loop 1 -i "$source" -t "$duration" \
    -vf "$(scene_filter "$label" "$framing")" -r 25 -an -c:v libx264 -preset medium -crf 18 \
    -pix_fmt yuv420p "$TMP/scene-$i.mp4"
}

idx=0
for scene in "${SCENES[@]}"; do
  IFS='|' read -r label stamp duration framing <<< "$scene"
  make_scene "$idx" "$label" "$stamp" "$duration" "$framing"
  idx=$((idx + 1))
done

printf "file '%s'\n" "$TMP"/scene-*.mp4 > "$TMP/concat.txt"
ffmpeg -hide_banner -loglevel error -y -f concat -safe 0 -i "$TMP/concat.txt" \
  -c copy -movflags +faststart "$MP4"

# A high-resolution GIF for GitHub's inline README playback. Palette generation
# materially improves text and dark UI edges compared with direct GIF encoding.
ffmpeg -hide_banner -loglevel error -y -i "$MP4" -vf "fps=10,scale=1280:-1:flags=lanczos,split[a][b];[a]palettegen=max_colors=256:stats_mode=diff[palette];[b][palette]paletteuse=dither=sierra2_4a" "$GIF"

# Use the final configured endpoint as the share-card thumbnail.
ffmpeg -hide_banner -loglevel error -y -ss 28 -i "$MP4" -frames:v 1 -q:v 2 "$THUMB"

echo "Created: $MP4"
echo "Created: $GIF"
echo "Created: $THUMB"
ffprobe -v error -show_entries stream=width,height,codec_name,avg_frame_rate:format=duration,size -of default=noprint_wrappers=1 "$MP4"
ffprobe -v error -show_entries stream=width,height,codec_name,avg_frame_rate:format=duration,size -of default=noprint_wrappers=1 "$GIF"
