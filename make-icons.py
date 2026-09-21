"""Rebuilds every icon from one small base64 text file.

The repository is maintained from a phone, where GitHub's uploader mangles
binary files and cannot create folders, so the logo lives as base64 text in
logo.b64 and every image the build needs is produced here.
"""

import base64
import io
import pathlib
import sys

try:
    from PIL import Image
except ImportError:
    sys.exit("Pillow is required: python -m pip install pillow")

root = pathlib.Path(__file__).resolve().parent
source = root / "logo.b64"
if not source.exists():
    sys.exit("logo.b64 is missing from the repository root")

raw = base64.b64decode("".join(source.read_text().split()))
logo = Image.open(io.BytesIO(raw)).convert("RGBA")
if logo.size[0] < 256:
    sys.exit(f"logo.b64 is only {logo.size[0]}px wide; 256px or more is needed")

icons = root / "src-tauri" / "icons"
icons.mkdir(parents=True, exist_ok=True)
(root / "src").mkdir(exist_ok=True)

for name, size in {
    "32x32.png": 32,
    "128x128.png": 128,
    "128x128@2x.png": 256,
    "icon.png": 512,
}.items():
    logo.resize((size, size), Image.LANCZOS).save(icons / name)
    print(f"  {name}")

# Windows needs a multi-resolution .ico for the window, taskbar and installer.
logo.resize((256, 256), Image.LANCZOS).save(
    icons / "icon.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)]
)
print("  icon.ico")

logo.resize((256, 256), Image.LANCZOS).save(root / "src" / "logo.png")
print("  src/logo.png")
