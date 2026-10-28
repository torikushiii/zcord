# Display-name font bundle

zcord bundles the 11 Discord display-name presets in this directory. They were
converted from the WOFF2 builds supplied for this task to SFNT (`.ttf`, or
`.otf` for the two CFF faces) with fontTools 4.46.0 using `font.flavor = None`.
The conversion changes only the container: glyph outlines, OpenType layout,
family/style names, internal weight values, and notices are preserved. The
supplied builds already carry Discord's display-name family names,
`Discord: ... 2025` unique IDs, and the Discord notice. No `gg sans` file is
bundled, and zcord does not download fonts at runtime.

Every file is licensed under the SIL Open Font License, Version 1.1. The full
license text is in [OFL.txt](OFL.txt) and at
<https://openfontlicense.org/open-font-license-official-text/>. Every file's
`name` table contains its original copyright notice followed by:

> Portions Copyright 2025 Discord Inc.
> https://discord.com/licenses

and links `name` ID 14 to the official OFL text above.

## Original notices

- **ID 3 / `Sakura.ttf` / loaded family `Sakura`**
  Original typeface: Cherry Bomb One.
  Original notice: Copyright 2019 The Cherry Bomb Project Authors
  (https://github.com/satsuyako/CherryBomb).

- **ID 4 / `Jellybean.ttf` / loaded family `Jellybean`**
  Original typeface: Chicle.
  Original notice: Copyright (c) 2007 Angel Koziupa
  (sudtipos@sudtipos.com), Copyright (c) 2007 Alejandro Paul
  (sudtipos@sudtipos.com), with Reserved Font Name "Chicle".

- **ID 6 / `Modern.otf` / loaded family `Modern`**
  Original typeface: MuseoModerno.
  Original notice: Copyright 2020 The MuseoModerno Project Authors
  (https://github.com/Omnibus-Type/MuseoModerno).

- **ID 7 / `Medieval.otf` / loaded family `Medieval`**
  Original typeface: Neo Castel.
  Original notice: Copyright 2023 Max Lillo.

- **ID 8 / `8Bit.ttf` / loaded family `8Bit`**
  Original typeface: Pixelify Sans.
  Original notice: Copyright 2021 The Pixelify Sans Project Authors
  (https://github.com/eifetx/Pixelify-Sans).

- **ID 10 / `Vampyre.ttf` / loaded family `Vampyre`**
  Original typeface: Sinistre.
  Original notice: © 2023, Jules Durand & Collletttivo.

- **ID 12 / `Tempo.ttf` / loaded family `Tempo`**
  Original typeface: Zilla Slab.
  Original notice: Copyright 2017, The Mozilla Foundation.

- **ID 13 / `Monkey-Bars.ttf` / loaded family `Monkey Bars`**
  Original typeface: Playpen Sans.
  Original notice: Copyright 2023 The Playpen Sans Project Authors
  (https://github.com/TypeTogether/Playpen-Sans).

- **ID 14 / `Mainframe.ttf` / loaded family `Mainframe`**
  Original typeface: Orbitron.
  Original notice: Copyright 2018 The Orbitron Project Authors
  (https://github.com/theleagueof/orbitron), with Reserved Font Name:
  "Orbitron".

- **ID 15 / `Headbang.ttf` / loaded family `Headbang`**
  Original typeface: New Rocker.
  Original notice: Copyright (c) 2012, Pablo Impallari
  (www.impallari.com|impallari@gmail.com), Copyright (c) 2012, Brenda Gallo
  (gbrenda1987@gmail.com), Copyright (c) 2012, Rodrigo Fuenzalida
  (www.rfuenzalida.com|hello@rfuenzalida.com), with Reserved Font Name
  'New Rocker'.

- **ID 16 / `Journal.ttf` / loaded family `Journal`**
  Original typeface: Kalam.
  Original notice: Copyright (c) 2014 Indian Type Foundry
  (info@indiantypefoundry.com).

## Licensing notes

The display-name family names are Discord's names for these modified builds.
Where an original OFL notice reserves a font name (for example "Chicle",
"Orbitron", or "New Rocker"), the redistributed build does not use that
reserved name as its primary family name. The original copyright holders and
authors are acknowledged only as required by the OFL; their names do not imply
endorsement of zcord or of Discord's modified builds.

The WOFF2 inputs were supplied for this effort and were not downloaded by
zcord. Discord's font licensing information is at
<https://discord.com/licenses>.
