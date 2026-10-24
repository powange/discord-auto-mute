"""Draws the application and tray icons (Pillow): python3 tools/make_icons.py"""

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
SCALE = 8  # supersampling, then downscaled for smooth edges

COLORS = {
    "ready": (88, 101, 242),  # Discord blurple: a profile is active, Discord live
    "muting": (237, 66, 69),  # Discord red: muted by the application
    "idle": (128, 132, 142),  # no game in the foreground
    "warning": (240, 160, 40),  # Discord or the keyboard cannot be reached
}


def draw(size: int, color, slashed=False, paused=False) -> Image.Image:
    s = size * SCALE
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    d.ellipse((0, 0, s - 1, s - 1), fill=color + (255,))
    white = (255, 255, 255, 255)
    u = s / 64
    if paused:
        d.rounded_rectangle((20 * u, 17 * u, 28 * u, 47 * u), radius=2 * u, fill=white)
        d.rounded_rectangle((36 * u, 17 * u, 44 * u, 47 * u), radius=2 * u, fill=white)
    else:
        d.rounded_rectangle((25 * u, 12 * u, 39 * u, 37 * u), radius=7 * u, fill=white)
        d.arc((18 * u, 20 * u, 46 * u, 44 * u), start=0, end=180, fill=white, width=int(4 * u))
        d.line((32 * u, 44 * u, 32 * u, 51 * u), fill=white, width=int(4 * u))
        d.line((24 * u, 52 * u, 40 * u, 52 * u), fill=white, width=int(4 * u))
        if slashed:
            d.line((16 * u, 14 * u, 48 * u, 52 * u), fill=color + (255,), width=int(9 * u))
            d.line((16 * u, 14 * u, 48 * u, 52 * u), fill=white, width=int(4 * u))
    return img.resize((size, size), Image.LANCZOS)


def main():
    (ROOT / "tray").mkdir(parents=True, exist_ok=True)
    draw(64, COLORS["ready"]).save(ROOT / "tray" / "ready.png")
    draw(64, COLORS["muting"], slashed=True).save(ROOT / "tray" / "muting.png")
    draw(64, COLORS["idle"]).save(ROOT / "tray" / "idle.png")
    draw(64, COLORS["idle"], paused=True).save(ROOT / "tray" / "paused.png")
    draw(64, COLORS["warning"]).save(ROOT / "tray" / "warning.png")
    for size, name in [(32, "32x32.png"), (128, "128x128.png"), (256, "128x128@2x.png"), (512, "icon.png")]:
        draw(size, COLORS["ready"]).save(ROOT / name)
    draw(256, COLORS["ready"]).save(ROOT / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])


if __name__ == "__main__":
    main()
