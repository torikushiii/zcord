import argparse
from pathlib import Path
import struct
import xml.etree.ElementTree as ET
import zlib


def pack(source: Path, destination: Path) -> None:
    files = sorted(source.glob("*.svg"), key=lambda path: path.stem)
    if not files:
        raise ValueError(f"no SVG files in {source}")
    index = bytearray()
    data = bytearray()
    samples = {0, len(files) // 2, len(files) - 1}
    for position, path in enumerate(files):
        name = path.stem.encode("ascii")
        if any(f"{int(part, 16):x}" != part for part in path.stem.split("-")):
            raise ValueError(f"noncanonical Twemoji name: {path.name}")
        svg = path.read_bytes()
        if not svg.startswith(b"<svg"):
            raise ValueError(f"expected an SVG root: {path.name}")
        root = ET.fromstring(svg)
        if root.tag != "{http://www.w3.org/2000/svg}svg":
            raise ValueError(f"unexpected root: {path.name}")
        attributes = b"".join(
            f' {attribute}="128"'.encode("ascii")
            for attribute in ("width", "height")
            if attribute not in root.attrib
        )
        svg = svg[:4] + attributes + svg[4:]
        ET.fromstring(svg)
        compressed = zlib.compress(svg, 9)
        if position in samples:
            restored = zlib.decompress(compressed)
            restored_root = ET.fromstring(restored)
            assert restored.startswith(b"<svg")
            assert restored_root.attrib["width"] == "128"
            assert restored_root.attrib["height"] == "128"
            print(f"Verified {path.name}: starts with <svg, XML parses, 128x128")
        index.extend(struct.pack("<H", len(name)))
        index.extend(name)
        index.extend(struct.pack("<II", len(data), len(compressed)))
        data.extend(compressed)
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(b"TWE1" + struct.pack("<I", len(files)) + index + data)
    print(f"Packed {len(files)} SVGs into {destination} ({destination.stat().st_size} bytes)")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path, help="Twemoji assets/svg directory")
    parser.add_argument(
        "destination",
        nargs="?",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "assets" / "twemoji.bin",
    )
    arguments = parser.parse_args()
    pack(arguments.source, arguments.destination)


if __name__ == "__main__":
    main()
