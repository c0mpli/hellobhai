#!/bin/bash
# Cuts one talking-avatar video (e.g. a free HeyGen export) into the clips
# the avatar bubble plays. Spoken parts become talking1...N; the longest
# pauses become idle, listening and thinking. Clips are cropped square,
# scaled to 480px and stripped of audio.
#
# Usage: ./scripts/make-avatar-clips.sh <video> <woman|man>
set -euo pipefail

input_video="$1"
character_name="$2"
output_directory="$HOME/Library/Application Support/hellobhai/Avatars/$character_name"
mkdir -p "$output_directory"

video_duration=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$input_video")

# Pauses of 1.5s+ quieter than -35dB, as "start end" lines.
silence_intervals=$(ffmpeg -hide_banner -nostats -i "$input_video" -af silencedetect=noise=-35dB:d=1.5 -f null - 2>&1 \
  | awk -v duration="$video_duration" '
      /silence_start/ { start = $NF }
      /silence_end/   { print start, $5; start = "" }
      END { if (start != "") print start, duration }')

if [ -z "$silence_intervals" ]; then
  echo "No pauses found. Add a few seconds of silence before and between the spoken lines." >&2
  exit 1
fi

# bc drops the leading zero (".2"), which ffmpeg rejects, so do math in awk.
calculate() { awk "BEGIN { printf \"%.2f\", $1 }"; }

cut_clip() {
  local clip_name="$1" clip_start="$2" clip_end="$3"
  ffmpeg -nostdin -hide_banner -loglevel error -y -ss "$clip_start" -to "$clip_end" -i "$input_video" \
    -vf "crop='min(iw,ih)':'min(iw,ih)',scale=480:480" -an -c:v libx264 -pix_fmt yuv420p -crf 20 \
    "$output_directory/$clip_name.mp4"
  echo "  $clip_name.mp4  ${clip_start}s → ${clip_end}s"
}

rm -f "$output_directory"/{idle,listening,thinking}.mp4 "$output_directory"/talking*.mp4
echo "Writing clips to $output_directory"

# Trim 0.2s off each pause edge so no lip movement leaks into silent clips.
# Longest pause → idle, next → listening, next → thinking.
silent_clip_names=(idle listening thinking)
silent_clip_index=0
while read -r pause_start pause_end; do
  [ $silent_clip_index -ge 3 ] && break
  cut_clip "${silent_clip_names[$silent_clip_index]}" \
    "$(calculate "$pause_start + 0.2")" "$(calculate "$pause_end - 0.2")"
  silent_clip_index=$((silent_clip_index + 1))
done < <(echo "$silence_intervals" | awk '{ print $1, $2, $2 - $1 }' | sort -k3 -nr | cut -d' ' -f1,2)

# Everything between pauses is speech; keep stretches of 2s or more.
talking_clip_number=1
previous_pause_end=0
while read -r pause_start pause_end; do
  if awk "BEGIN { exit !($pause_start - $previous_pause_end >= 2) }"; then
    cut_clip "talking$talking_clip_number" "$previous_pause_end" "$pause_start"
    talking_clip_number=$((talking_clip_number + 1))
  fi
  previous_pause_end="$pause_end"
done < <(echo "$silence_intervals"; echo "$video_duration $video_duration")

echo "Done. Pick \"$character_name\" under Avatar in the hellobhai menu bar panel (or relaunch)."
