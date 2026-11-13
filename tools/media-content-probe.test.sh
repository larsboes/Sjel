#!/usr/bin/env bash
# Native Media evidence contract: local files, partial failures, bounded video samples.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [ "$(uname -s)" != Darwin ]; then
  echo "SKIP media-content-probe: Apple Vision requires macOS"
  exit 0
fi
SCRATCH="$(mktemp -d /tmp/sjel-media-probe-test.XXXXXX)"
SCRATCH="$(cd "$SCRATCH" && pwd -P)"
trap 'rm -rf "$SCRATCH"' EXIT
PROBE="$ROOT/tools/media-content-probe"
check_previews() {
  swift - "$1" <<'SWIFT'
import Foundation
import ImageIO

let report = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
let object = try JSONSerialization.jsonObject(with: report) as! [String: Any]
let items = object["items"] as! [[String: Any]]
var count = 0
for item in items {
    for frame in item["frames"] as! [[String: Any]] {
        guard let encoded = frame["previewPng"] as? String,
              let data = Data(base64Encoded: encoded),
              data.count <= 512 * 1024,
              data.starts(with: [137, 80, 78, 71, 13, 10, 26, 10]),
              let source = CGImageSourceCreateWithData(data as CFData, nil),
              let image = CGImageSourceCreateImageAtIndex(source, 0, nil),
              image.width > 0 && image.width <= 256,
              image.height > 0 && image.height <= 256 else {
            fatalError("Missing, invalid or oversized PNG preview")
        }
        count += 1
    }
}
precondition(count > 0, "No previews checked")
SWIFT
}
cp "$ROOT/crates/extraction/eval/fixtures/en-prose.png" "$SCRATCH/original"
printf 'not media\n' > "$SCRATCH/text"
touch "$SCRATCH/empty"
dd if=/dev/zero of="$SCRATCH/oversized" bs=1 count=0 seek=1073741825 2>/dev/null
export TMPDIR="$SCRATCH/"
ln -s "$SCRATCH/original" "$SCRATCH/link"
ln -s "$SCRATCH" "$SCRATCH/linked-parent"
before="$(shasum -a 256 "$SCRATCH/original")"
if "$PROBE" "$SCRATCH/original" "$SCRATCH/text" "$SCRATCH/link" \
    "$SCRATCH/linked-parent/original" relative-file "$SCRATCH/../invalid" \
    "$SCRATCH/empty" "$SCRATCH/oversized" "$SCRATCH" \
    > "$SCRATCH/report.json"; then
  echo "FAIL: a partial batch returned success" >&2; exit 1
fi
jq -e '
  .formatVersion == 1 and
  .engine == "apple-vision/VNClassifyImageRequest" and .revision > 0 and
  (.items | length) == 9 and
  .items[0].kind == "image" and (.items[0].sha256 | test("^[a-f0-9]{64}$")) and
  (.items[0].frames | length) == 1 and
  (.items[0].frames[0].labels | length) > 0 and
  all(.items[0].frames[0].labels[]; .score >= 0 and .score <= 1) and
  all(.items[1:][]; .error != null and .sha256 == null and (.frames | length) == 0)
' "$SCRATCH/report.json" >/dev/null
check_previews "$SCRATCH/report.json"
[ -z "$(find "$SCRATCH" -mindepth 1 -maxdepth 1 -type d -name 'sjel-media-probe-*' -print -quit)" ]
[ "$(shasum -a 256 "$SCRATCH/original")" = "$before" ]
[ "$(jq -r '.items[0].sha256' "$SCRATCH/report.json")" = "${before%% *}" ]
if "$PROBE" >/dev/null 2>&1; then echo "FAIL: empty batch accepted" >&2; exit 1; fi
set --
for _ in {1..21}; do set -- "$@" "$SCRATCH/original"; done
if "$PROBE" "$@" >/dev/null 2>&1; then echo "FAIL: oversized batch accepted" >&2; exit 1; fi
if command -v ffmpeg >/dev/null 2>&1; then
  ffmpeg -v error -f lavfi -i 'testsrc=size=160x120:rate=10' -t 10 \
    -c:v libx264 -g 300 -pix_fmt yuv420p "$SCRATCH/video.mp4"
  mv "$SCRATCH/video.mp4" "$SCRATCH/extensionless-video"
  video_before="$(shasum -a 256 "$SCRATCH/extensionless-video")"
  "$PROBE" "$SCRATCH/extensionless-video" > "$SCRATCH/video.json"
  jq -e '.formatVersion == 1 and
    .items[0].kind == "video" and (.items[0].frames | length) == 3 and
    all(.items[0].frames[]; .seconds >= 0 and (.labels | length) > 0 and
      ((.seconds - .requestedSeconds) | fabs) <= 0.15) and
    ([.items[0].frames[].requestedSeconds] == [1,5,9]) and
    (.items[0].frames[0].seconds < .items[0].frames[1].seconds) and
    (.items[0].frames[1].seconds < .items[0].frames[2].seconds)' \
    "$SCRATCH/video.json" >/dev/null
  check_previews "$SCRATCH/video.json"
  [ -z "$(find "$SCRATCH" -mindepth 1 -maxdepth 1 -type d -name 'sjel-media-probe-*' -print -quit)" ]
  [ "$(shasum -a 256 "$SCRATCH/extensionless-video")" = "$video_before" ]
else
  echo "SKIP media-content-probe video fixture: optional ffmpeg is absent"
fi
echo "PASS media-content-probe: versioned report, PNG previews, extensionless image, digest, errors, symlink and traversal refusal, limits, source preservation"
