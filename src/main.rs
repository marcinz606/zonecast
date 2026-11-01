use std::{sync::mpsc, thread, time::Duration};

use jiff::{Timestamp, civil::Date, tz::TimeZone};
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};
use serde::Deserialize;

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const REFRESH: Duration = Duration::from_secs(15 * 60);
const CARD_WIDTH: u16 = 38;
const CARD_HEIGHT: u16 = 12;

#[derive(Deserialize)]
struct Place {
    name: String,
    latitude: f64,
    longitude: f64,
    timezone: String,
    #[serde(default)]
    country: String,
}

#[derive(Deserialize)]
struct Geo {
    results: Option<Vec<Place>>,
}

#[derive(Deserialize)]
struct Current {
    temperature_2m: f64,
    weather_code: u8,
    is_day: u8,
}

#[derive(Deserialize)]
struct Daily {
    time: Vec<String>,
    weather_code: Vec<u8>,
    temperature_2m_max: Vec<f64>,
    temperature_2m_min: Vec<f64>,
    sunrise: Vec<String>,
    sunset: Vec<String>,
}

#[derive(Deserialize)]
struct Weather {
    current: Current,
    daily: Daily,
}

struct City {
    name: String,
    country: String,
    tz: TimeZone,
    coords: (f64, f64),
}

/// Makes each country contiguous; countries keep the order of their first city.
fn group_by_country(cities: &mut [City]) {
    let mut order: Vec<String> = Vec::new();
    for c in cities.iter() {
        if !order.contains(&c.country) {
            order.push(c.country.clone());
        }
    }
    cities.sort_by_key(|c| order.iter().position(|o| *o == c.country));
}

#[derive(Default)]
struct Slot {
    weather: Option<Weather>,
    error: bool,
}

fn get(url: &str, query: &[(&str, &str)]) -> Res<String> {
    let mut req = ureq::get(url);
    for (k, v) in query {
        req = req.query(*k, *v);
    }
    Ok(req.call()?.body_mut().read_to_string()?)
}

fn geocode(name: &str) -> Res<City> {
    let body = get(
        "https://geocoding-api.open-meteo.com/v1/search",
        &[("name", name), ("count", "1")],
    )?;
    let geo: Geo = serde_json::from_str(&body)?;
    let p = geo
        .results
        .and_then(|r| r.into_iter().next())
        .ok_or_else(|| format!("city not found: {name}"))?;
    Ok(City {
        tz: TimeZone::get(&p.timezone)?,
        name: p.name,
        country: p.country,
        coords: (p.latitude, p.longitude),
    })
}

fn fetch_weather((lat, lon): (f64, f64)) -> Res<Weather> {
    let body = get(
        "https://api.open-meteo.com/v1/forecast",
        &[
            ("latitude", &lat.to_string()),
            ("longitude", &lon.to_string()),
            ("current", "temperature_2m,weather_code,is_day"),
            (
                "daily",
                "weather_code,temperature_2m_max,temperature_2m_min,sunrise,sunset",
            ),
            ("forecast_days", "4"),
            ("timezone", "auto"),
        ],
    )?;
    Ok(serde_json::from_str(&body)?)
}

/// WMO weather code to a one-cell glyph and its color.
fn icon(code: u8) -> (&'static str, Color) {
    match code {
        0 | 1 => ("☀", Color::Yellow),
        2 | 3 => ("☁", Color::Gray),
        45 | 48 => ("≡", Color::DarkGray),
        51..=67 | 80..=82 => ("☂", Color::Cyan),
        71..=77 | 85 | 86 => ("❄", Color::White),
        95..=99 => ("⚡", Color::Magenta),
        _ => ("?", Color::DarkGray),
    }
}

fn deg(t: f64) -> i32 {
    t.round() as i32
}

/// Cold-to-hot gradient, linear between the stops and clamped at both ends.
fn temp_color(t: f64) -> Color {
    const STOPS: [(f64, [u8; 3]); 5] = [
        (-10.0, [90, 130, 255]),
        (0.0, [80, 200, 230]),
        (10.0, [110, 200, 120]),
        (20.0, [240, 200, 80]),
        (30.0, [240, 90, 60]),
    ];
    let t = t.clamp(STOPS[0].0, STOPS[4].0);
    let k = STOPS.windows(2).position(|s| t <= s[1].0).unwrap_or(3);
    let ((t0, c0), (t1, c1)) = (STOPS[k], STOPS[k + 1]);
    let a = (t - t0) / (t1 - t0);
    let mix = |i: usize| (c0[i] as f64 + (c1[i] as f64 - c0[i] as f64) * a).round() as u8;
    Color::Rgb(mix(0), mix(1), mix(2))
}

/// Renders digits and `:` as three rows of half-block seven-segment glyphs.
fn segments(time: &str) -> [String; 3] {
    const DIGITS: [[&str; 3]; 10] = [
        ["█▀█", "█ █", "▀▀▀"],
        ["  █", "  █", "  ▀"],
        ["▀▀█", "█▀▀", "▀▀▀"],
        ["▀▀█", " ▀█", "▀▀▀"],
        ["█ █", "▀▀█", "  ▀"],
        ["█▀▀", "▀▀█", "▀▀▀"],
        ["█▀▀", "█▀█", "▀▀▀"],
        ["▀▀█", "  █", "  ▀"],
        ["█▀█", "█▀█", "▀▀▀"],
        ["█▀█", "▀▀█", "▀▀▀"],
    ];
    let glyphs: Vec<[&str; 3]> = time
        .chars()
        .map(|c| match c.to_digit(10) {
            Some(d) => DIGITS[d as usize],
            None => ["▄", "▄", " "],
        })
        .collect();
    std::array::from_fn(|row| glyphs.iter().map(|g| g[row]).collect::<Vec<_>>().join(" "))
}

/// "2026-11-01T06:41" -> "06:41".
fn clock_part(iso: &str) -> &str {
    iso.split_once('T').map_or(iso, |(_, t)| t)
}

/// Sections keep full height: when they overflow, `scroll` (clamped here) picks the first one shown.
fn draw(
    f: &mut Frame,
    cities: &[City],
    slots: &[Slot],
    updated: Option<Timestamp>,
    scroll: &mut usize,
) {
    let [body, footer] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(f.area());
    let cols = ((body.width / CARD_WIDTH) as usize).max(1);
    let mut groups = Vec::new();
    let mut start = 0;
    for g in cities.chunk_by(|a, b| a.country == b.country) {
        groups.push(start..start + g.len());
        start += g.len();
    }
    let height = |g: &std::ops::Range<usize>| 1 + g.len().div_ceil(cols) as u16 * CARD_HEIGHT;
    let total = groups.iter().map(height).sum::<u16>() + groups.len() as u16 - 1;
    let fits = total <= body.height;
    *scroll = if fits {
        0
    } else {
        (*scroll).min(groups.len() - 1)
    };

    let mut y = body.y + body.height.saturating_sub(total) / 2;
    for (k, g) in groups.into_iter().enumerate().skip(*scroll) {
        let h = height(&g);
        if k > *scroll && y + h > body.bottom() {
            break;
        }
        let section = Rect::new(body.x, y, body.width, h).intersection(body);
        y += h + 1;
        let width = cols.min(g.len()) as u16 * CARD_WIDTH;
        let [header, grid] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(section);
        let [header] = Layout::horizontal([Constraint::Length(width)])
            .flex(Flex::Center)
            .areas(header);
        let country = &cities[g.start].country;
        let rule = "─".repeat((width as usize).saturating_sub(country.chars().count() + 4));
        let title = Line::from(vec![
            "── ".dim(),
            country.as_str().bold(),
            format!(" {rule}").dim(),
        ]);
        f.render_widget(title, header);
        draw_grid(f, grid, &cities[g.clone()], &slots[g], cols);
    }

    let status = match updated {
        Some(t) => format!(
            "updated {}",
            t.to_zoned(TimeZone::system()).strftime("%H:%M")
        ),
        None => "updating…".into(),
    };
    let mut keys = vec![" q".bold(), " quit  ".dim(), "r".bold(), " refresh".dim()];
    if !fits {
        keys.extend(["  ↑↓".bold(), " scroll".dim()]);
    }
    let keys = Line::from(keys);
    f.render_widget(keys, footer);
    f.render_widget(
        Line::from(format!("{status} ").dim()).right_aligned(),
        footer,
    );
}

fn draw_grid(f: &mut Frame, area: Rect, cities: &[City], slots: &[Slot], cols: usize) {
    let n = cities.len();
    let rows =
        Layout::vertical(vec![Constraint::Length(CARD_HEIGHT); n.div_ceil(cols)]).split(area);
    for (r, row) in rows.iter().enumerate() {
        let first = r * cols;
        let count = cols.min(n - first);
        let cells = Layout::horizontal(vec![Constraint::Length(CARD_WIDTH); count])
            .flex(Flex::Center)
            .split(*row);
        for (c, cell) in cells.iter().enumerate() {
            draw_card(f, *cell, &cities[first + c], &slots[first + c]);
        }
    }
}

fn draw_card(f: &mut Frame, area: Rect, city: &City, slot: &Slot) {
    let now = Timestamp::now().to_zoned(city.tz.clone());
    let day = slot.weather.as_ref().is_none_or(|w| w.current.is_day == 1);
    let accent = if day { Color::Yellow } else { Color::Blue };
    let border = slot
        .weather
        .as_ref()
        .map_or(Color::DarkGray, |w| temp_color(w.current.temperature_2m));

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(format!(" {} ", city.name)).bold())
        .title(Line::from(format!(" {} ", now.strftime("UTC%:z"))).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [_, clock, _, info, sun, _, days] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .areas(inner);

    let big = segments(&now.strftime("%H:%M").to_string()).map(Line::from);
    f.render_widget(
        Paragraph::new(big.to_vec())
            .centered()
            .style(Style::new().fg(accent)),
        clock,
    );
    f.render_widget(Line::from(format!(" {}", now.strftime("%a %-d %b"))), info);

    let Some(w) = &slot.weather else {
        let msg = if slot.error {
            "weather unavailable ".red()
        } else {
            "loading… ".dim()
        };
        f.render_widget(Line::from(msg).right_aligned(), info);
        return;
    };

    let (glyph, color) = icon(w.current.weather_code);
    let d = &w.daily;
    let mut now_spans = vec![
        Span::styled(glyph, color),
        format!(" {}°C", deg(w.current.temperature_2m)).bold(),
    ];
    if let (Some(hi), Some(lo)) = (d.temperature_2m_max.first(), d.temperature_2m_min.first()) {
        now_spans.push(format!("  ↑{}° ↓{}°", deg(*hi), deg(*lo)).dim());
    }
    if slot.error {
        now_spans.push(" (stale)".dim());
    }
    now_spans.push(" ".into());
    f.render_widget(Line::from(now_spans).right_aligned(), info);

    if let (Some(rise), Some(set)) = (d.sunrise.first(), d.sunset.first()) {
        let line = Line::from(vec![
            "sunrise ".dim(),
            clock_part(rise).into(),
            "   sunset ".dim(),
            clock_part(set).into(),
        ]);
        f.render_widget(line.centered(), sun);
    }

    let cols: [Rect; 3] = Layout::horizontal([Constraint::Ratio(1, 3); 3]).areas(days);
    for (i, col) in (1..d.time.len().min(4)).zip(cols) {
        let name = d.time[i]
            .parse::<Date>()
            .map(|t| t.strftime("%a").to_string())
            .unwrap_or_default();
        let (glyph, color) = icon(d.weather_code[i]);
        let text = vec![
            Line::from(name.dim()),
            Line::from(vec![
                Span::styled(glyph, color),
                format!(" {}°", deg(d.temperature_2m_max[i])).bold(),
                format!(" {}°", deg(d.temperature_2m_min[i])).dim(),
            ]),
        ];
        f.render_widget(Paragraph::new(text).centered(), col);
    }
}

fn cities_file() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(Into::into)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    Some(base.join("zonecast/cities"))
}

/// One city per line; blank lines and `#` comments are skipped.
fn parse_cities(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

fn main() -> Res<()> {
    let mut names: Vec<String> = std::env::args().skip(1).collect();
    let file = cities_file();
    if names.is_empty()
        && let Some(text) = file.as_ref().and_then(|p| std::fs::read_to_string(p).ok())
    {
        names = parse_cities(&text);
    }
    if names.is_empty() {
        eprintln!("usage: zonecast CITY [CITY...]   e.g. zonecast Warsaw Tokyo \"New York\"");
        if let Some(p) = file {
            eprintln!("or list cities in {}, one per line", p.display());
        }
        std::process::exit(2);
    }
    let mut cities = names.iter().map(|n| geocode(n)).collect::<Res<Vec<_>>>()?;
    group_by_country(&mut cities);

    let (tx, rx) = mpsc::channel();
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();
    let coords: Vec<_> = cities.iter().map(|c| c.coords).collect();
    thread::spawn(move || {
        loop {
            for (i, c) in coords.iter().enumerate() {
                if tx.send((i, fetch_weather(*c))).is_err() {
                    return;
                }
            }
            if let Err(mpsc::RecvTimeoutError::Disconnected) = refresh_rx.recv_timeout(REFRESH) {
                return;
            }
        }
    });

    let mut slots: Vec<Slot> = cities.iter().map(|_| Slot::default()).collect();
    let mut updated = None;
    let mut scroll = 0;
    let mut terminal = ratatui::init();
    let result = (|| -> Res<()> {
        loop {
            while let Ok((i, r)) = rx.try_recv() {
                match r {
                    Ok(w) => {
                        slots[i] = Slot {
                            weather: Some(w),
                            error: false,
                        };
                        updated = Some(Timestamp::now());
                    }
                    Err(_) => slots[i].error = true,
                }
            }
            terminal.draw(|f| draw(f, &cities, &slots, updated, &mut scroll))?;
            if event::poll(Duration::from_millis(500))?
                && let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
            {
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => _ = refresh_tx.send(()),
                    KeyCode::Down | KeyCode::Char('j') => scroll += 1,
                    KeyCode::Up | KeyCode::Char('k') => scroll = scroll.saturating_sub(1),
                    _ => {}
                }
            }
        }
    })();
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_forecast_and_maps_icons() {
        let json = r#"{
            "current": {"temperature_2m": -0.4, "weather_code": 71, "is_day": 0},
            "daily": {"time": ["2026-11-01", "2026-11-02"], "weather_code": [0, 95],
                      "temperature_2m_max": [3.6, 5.0], "temperature_2m_min": [-2.5, 1.0],
                      "sunrise": ["2026-11-01T06:41", "2026-11-02T06:43"],
                      "sunset": ["2026-11-01T16:23", "2026-11-02T16:21"]}
        }"#;
        let w: Weather = serde_json::from_str(json).unwrap();
        assert_eq!(deg(w.current.temperature_2m), 0);
        assert_eq!(icon(w.current.weather_code).0, "❄");
        assert_eq!(icon(w.daily.weather_code[1]).0, "⚡");
        assert_eq!(icon(2).0, "☁");
        assert_eq!(icon(200).0, "?");
    }

    #[test]
    fn sun_times_and_temp_colors() {
        assert_eq!(clock_part("2026-11-01T06:41"), "06:41");
        assert_eq!(clock_part("garbage"), "garbage");
        assert_eq!(temp_color(-40.0), Color::Rgb(90, 130, 255));
        assert_eq!(temp_color(45.0), Color::Rgb(240, 90, 60));
        assert_eq!(temp_color(10.0), Color::Rgb(110, 200, 120));
        assert_eq!(temp_color(15.0), Color::Rgb(175, 200, 100));
    }

    #[test]
    fn groups_cities_by_first_seen_country() {
        let city = |name: &str, country: &str| City {
            name: name.into(),
            country: country.into(),
            tz: TimeZone::UTC,
            coords: (0.0, 0.0),
        };
        let mut cities = [
            city("Warsaw", "Poland"),
            city("Tokyo", "Japan"),
            city("Krakow", "Poland"),
            city("Osaka", "Japan"),
            city("Lisbon", "Portugal"),
        ];
        group_by_country(&mut cities);
        let names: Vec<_> = cities.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Warsaw", "Krakow", "Tokyo", "Osaka", "Lisbon"]);
    }

    #[test]
    fn renders_segment_clock() {
        assert_eq!(
            segments("08:52"),
            [
                "█▀█ █▀█ ▄ █▀▀ ▀▀█",
                "█ █ █▀█ ▄ ▀▀█ █▀▀",
                "▀▀▀ ▀▀▀   ▀▀▀ ▀▀▀",
            ]
        );
    }

    #[test]
    fn parses_cities_file() {
        let text = "Warsaw\n\n  New York  \n# Lisbon\nTokyo\n";
        assert_eq!(parse_cities(text), ["Warsaw", "New York", "Tokyo"]);
    }
}
