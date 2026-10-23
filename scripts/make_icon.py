import argparse
from pathlib import Path

from PIL import Image

SIZES = [16, 20, 24, 32, 40, 48, 64, 128, 256]


def make(source: Path, destination: Path, margin: float) -> None:
    image = Image.open(source).convert("RGBA")
    alpha = image.getchannel("A").point(lambda value: value if value > 8 else 0)
    image.putalpha(alpha)
    left, top, right, bottom = alpha.getbbox()
    side = round(max(right - left, bottom - top) * (1 + 2 * margin))
    center_x, center_y = (left + right) / 2, (top + bottom) / 2
    square = Image.new("RGBA", (side, side))
    square.paste(image, (round(side / 2 - center_x), round(side / 2 - center_y)))
    square.save(destination, sizes=[(size, size) for size in SIZES])
    print(f"Wrote {destination} ({side}px crop, sizes {SIZES})")


def main() -> None:
    root = Path(__file__).resolve().parent.parent / "assets"
    parser = argparse.ArgumentParser(description="Build the Windows app icon from the logo.")
    parser.add_argument("source", type=Path, nargs="?", default=root / "logo.png")
    parser.add_argument("destination", type=Path, nargs="?", default=root / "zcord.ico")
    parser.add_argument("--margin", type=float, default=0.06)
    arguments = parser.parse_args()
    make(arguments.source, arguments.destination, arguments.margin)


if __name__ == "__main__":
    main()
