import argparse
import json
from pathlib import Path
import re


def emoji_table(bundle: str) -> list[dict]:
    match = re.search(r"JSON\.parse\('(\{\"emojis\":\[.*?)'\)", bundle, re.DOTALL)
    if not match:
        raise ValueError("emoji table not found in bundle")
    literal = match.group(1).replace("\\'", "'")
    decoded = literal.encode("ascii").decode("unicode_escape")
    decoded = decoded.encode("utf-16", "surrogatepass").decode("utf-16")
    return json.loads(decoded)["emojis"]


def pack(source: Path, destination: Path) -> None:
    emojis = emoji_table(source.read_text(encoding="utf-8"))
    lines = {}
    for emoji in emojis:
        for name in emoji["names"]:
            if "\t" in name or "\n" in name or ":" in name:
                raise ValueError(f"unexpected shortcode: {name!r}")
            value = emoji["surrogates"]
            if len(value) == 1 and ord(value) < 0x1F000:
                value += "\ufe0f"
            lines.setdefault(name, value)
    destination.write_text(
        "".join(f"{name}\t{value}\n" for name, value in sorted(lines.items())),
        encoding="utf-8",
    )
    print(f"Wrote {len(lines)} shortcodes for {len(emojis)} emoji to {destination}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Extract Discord's emoji shortcodes from its web client's vnd-emoji bundle."
    )
    parser.add_argument("bundle", type=Path, help="vnd-emoji.*.js from discord.com/assets")
    parser.add_argument(
        "destination",
        type=Path,
        nargs="?",
        default=Path(__file__).resolve().parent.parent / "assets" / "emoji_shortcodes.tsv",
    )
    arguments = parser.parse_args()
    pack(arguments.bundle, arguments.destination)


if __name__ == "__main__":
    main()
