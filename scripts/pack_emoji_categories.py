import argparse
from pathlib import Path
import struct
import tempfile
import urllib.request

# emoji-test 17.0; the historical short path /Public/emoji/17.0/ currently 404s.
SOURCE = "https://unicode.org/Public/17.0.0/emoji/emoji-test.txt"

GROUPS = {
    "Smileys & Emotion": "People",
    "People & Body": "People",
    "Animals & Nature": "Nature",
    "Food & Drink": "Food",
    "Activities": "Activities",
    "Travel & Places": "Travel",
    "Objects": "Objects",
    "Symbols": "Symbols",
    "Flags": "Flags",
}
SKIN_TONES = range(0x1F3FB, 0x1F400)


def twemoji_sequences(path: Path) -> set[str]:
    data = path.read_bytes()
    if data[:4] != b"TWE1":
        raise ValueError(f"unexpected Twemoji bundle header: {path}")
    count = struct.unpack_from("<I", data, 4)[0]
    offset = 8
    sequences = set()
    for _ in range(count):
        name_length = struct.unpack_from("<H", data, offset)[0]
        offset += 2
        name = data[offset : offset + name_length].decode("ascii")
        offset += name_length + 8
        sequence = "".join(chr(int(part, 16)) for part in name.split("-"))
        sequences.add(sequence.replace("\ufe0f", ""))
    return sequences


def shortcodes(path: Path) -> dict[str, str]:
    table = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        if "\t" not in line:
            continue
        name, emoji = line.split("\t", 1)
        table.setdefault(emoji.replace("\ufe0f", ""), name)
    return table


def pack(source: Path, twemoji: Path, shortcodes_path: Path, destination: Path) -> None:
    sequences = twemoji_sequences(twemoji)
    names = shortcodes(shortcodes_path)
    lines = []
    group = None
    for line in source.read_text(encoding="utf-8").splitlines():
        if line.startswith("# group: "):
            group = line[len("# group: ") :]
            continue
        if not line or line.startswith("#") or ";" not in line:
            continue
        code_points, rest = line.split(";", 1)
        if rest.split("#", 1)[0].strip() != "fully-qualified":
            continue
        emoji = "".join(chr(int(value, 16)) for value in code_points.split())
        if any(ord(c) in SKIN_TONES for c in emoji):
            continue
        key = emoji.replace("\ufe0f", "")
        category = GROUPS.get(group)
        if category is None or key not in names or key not in sequences:
            continue
        lines.append(f"{category}\t{emoji}\n")
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_text("".join(lines), encoding="utf-8")
    print(f"Wrote {len(lines)} fully-qualified emoji to {destination}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "source",
        type=Path,
        nargs="?",
        help="emoji-test.txt (downloaded to a temp dir when omitted)",
    )
    parser.add_argument(
        "twemoji",
        type=Path,
        nargs="?",
        default=Path(__file__).resolve().parents[1] / "assets" / "twemoji.bin",
    )
    parser.add_argument(
        "shortcodes",
        type=Path,
        nargs="?",
        default=Path(__file__).resolve().parents[1] / "assets" / "emoji_shortcodes.tsv",
    )
    parser.add_argument(
        "destination",
        type=Path,
        nargs="?",
        default=Path(__file__).resolve().parents[1] / "assets" / "emoji_categories.tsv",
    )
    arguments = parser.parse_args()

    if arguments.source is not None:
        source = arguments.source
        pack(source, arguments.twemoji, arguments.shortcodes, arguments.destination)
        return
    with tempfile.TemporaryDirectory() as directory:
        source = Path(directory) / "emoji-test.txt"
        urllib.request.urlretrieve(SOURCE, source)
        pack(source, arguments.twemoji, arguments.shortcodes, arguments.destination)


if __name__ == "__main__":
    main()
