#!/bin/bash
# Makes the avatar clips from one face photo and one "driving" video of you,
# entirely on this Mac: LivePortrait (open source) moves the face in the
# photo the way your face moves in the video, then make-avatar-clips.sh cuts
# the result into idle / listening / thinking / talking clips.
#
# Record the driving video with Photo Booth or QuickTime: ~8s looking at the
# camera quietly, then a few short spoken lines with a 3s pause after each.
#
# Usage: ./scripts/make-avatar-from-photo.sh <face-photo> <driving-video> <woman|man>
#
# First run installs LivePortrait into build/LivePortrait (~3GB: PyTorch and
# model weights). Its face detector (InsightFace) is licensed for
# non-commercial use only, so revisit this before shipping commercially.
set -euo pipefail

face_photo="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
driving_video="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
character_name="$3"

repository_root="$(cd "$(dirname "$0")/.." && pwd)"
liveportrait_directory="$repository_root/build/LivePortrait"
liveportrait_python="$liveportrait_directory/.venv/bin/python"

if [ ! -f "$liveportrait_directory/pretrained_weights/liveportrait/landmark.onnx" ]; then
  echo "Installing LivePortrait (one time)..."
  [ -d "$liveportrait_directory" ] || git clone --depth 1 https://github.com/KwaiVGI/LivePortrait.git "$liveportrait_directory"
  uv venv -q -p 3.10 "$liveportrait_directory/.venv"
  # On Apple Silicon these torch wheels come straight from PyPI; the CPU
  # index in requirements_macOS.txt is not needed.
  uv pip install -q -p "$liveportrait_python" -r "$liveportrait_directory/requirements_base.txt" \
    torch==2.3.0 torchvision==0.18.0 torchaudio==2.3.0 onnxruntime-silicon==1.16.3 requests "huggingface_hub[cli]"
  "$liveportrait_directory/.venv/bin/hf" download KwaiVGI/LivePortrait --local-dir "$liveportrait_directory/pretrained_weights"
fi

work_directory="$(mktemp -d)"
trap 'rm -rf "$work_directory"' EXIT

# LivePortrait takes ~3s per frame on an M3; 15fps halves the render time
# and still looks smooth at bubble size.
ffmpeg -nostdin -hide_banner -loglevel error -y -i "$driving_video" -r 15 "$work_directory/driving.mp4"

echo "Animating the photo (roughly 6 minutes per 10 seconds of video on an M3)..."
(cd "$liveportrait_directory" && PYTORCH_ENABLE_MPS_FALLBACK=1 "$liveportrait_python" inference.py \
  -s "$face_photo" -d "$work_directory/driving.mp4" -o "$work_directory/animated" > "$work_directory/liveportrait.log" 2>&1) \
  || { tail -20 "$work_directory/liveportrait.log"; exit 1; }

animated_video=$(ls "$work_directory"/animated/*.mp4 | grep -v _concat | head -1)

# The pause detection in make-avatar-clips.sh needs the driving video's
# audio, so copy it onto the animated video.
ffmpeg -nostdin -hide_banner -loglevel error -y -i "$animated_video" -i "$driving_video" \
  -map 0:v -map 1:a -c:v copy -shortest "$work_directory/animated_with_audio.mp4"

"$repository_root/scripts/make-avatar-clips.sh" "$work_directory/animated_with_audio.mp4" "$character_name"
