# zonecast

A terminal app that shows world clocks and the weather for the cities you choose.

```
── Poland ──────────────────────────────────────────────────────────────────
╭ Warsaw ───────────────── UTC+01:00 ╮╭ Krakow ───────────────── UTC+01:00 ╮
│                                    ││                                    │
│          █▀█ █▀█ ▄   █   █         ││          █▀█ █▀█ ▄   █   █         │
│          █ █ ▀▀█ ▄   █   █         ││          █ █ ▀▀█ ▄   █   █         │
│          ▀▀▀ ▀▀▀     ▀   ▀         ││          ▀▀▀ ▀▀▀     ▀   ▀         │
│                                    ││                                    │
│ Sun 1 Nov           ☁ 6°C  ↑8° ↓6° ││ Sun 1 Nov           ☂ 7°C  ↑8° ↓6° │
│    sunrise 06:31   sunset 16:07    ││    sunrise 06:29   sunset 16:17    │
│                                    ││                                    │
│     Mon         Tue         Wed    ││     Mon         Tue         Wed    │
│   ☁ 8° 6°    ☁ 10° 4°     ☁ 8° 0°  ││   ☂ 8° 5°     ☂ 8° 3°     ☂ 8° 3°  │
╰────────────────────────────────────╯╰────────────────────────────────────╯
```

Each city card shows:

- the local time and UTC offset,
- the current weather, with the high and low temperature for today,
- the sunrise and sunset times,
- the forecast for the next 3 days.

Cards are grouped by country. The border color shows the temperature, from blue (cold) to red (hot). The clock is yellow in daytime and blue at night.

Weather data comes from [Open-Meteo](https://open-meteo.com). It is free and needs no API key. The weather updates every 15 minutes.

## Run

You need Rust 1.88 or later.

```bash
cargo run --release -- Warsaw Tokyo "New York"
```

To install the binary:

```bash
cargo install --path .
zonecast Warsaw Tokyo "New York"
```

## Cities file

If you give no cities as arguments, zonecast reads `~/.config/zonecast/cities` (or `$XDG_CONFIG_HOME/zonecast/cities`). Write one city per line. Lines that start with `#` are skipped.

```
Warsaw
Tokyo
New York
# Lisbon
```

## Keys

| Key | Action |
|---|---|
| `q`, `Esc` | Quit |
| `r` | Update the weather now |
| `↑` `↓`, `k` `j` | Scroll when the cards do not fit the screen |
