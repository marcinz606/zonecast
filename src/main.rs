use std::{sync::mpsc, thread, time::Duration};

use jiff::{Timestamp, civil::Date, tz::TimeZone};
use ratatui::{
    Frame,
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};
use serde::Deserialize;
use tui_big_text::{BigText, PixelSize};

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const REFRESH: Duration = Duration::from_secs(15 * 60);
const CARD_WIDTH: u16 = 38;
const CARD_HEIGHT: u16 = 11;

#[derive(Deserialize)]
struct Place {
    name: String,
    latitude: f64,
    longitude: f64,
    timezone: String,
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
}

#[derive(Deserialize)]
struct Weather {
    current: Current,
    daily: Daily,
}

struct City {
    name: String,
    tz: TimeZone,
    coords: (f64, f64),
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
                "weather_code,temperature_2m_max,temperature_2m_min",
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

fn draw(f: &mut Frame, cities: &[City], slots: &[Slot]) {
    let area = f.area();
    let cols = ((area.width / CARD_WIDTH) as usize).clamp(1, cities.len());
    let rows = cities.len().div_ceil(cols);
    let row_areas = Layout::vertical(vec![Constraint::Length(CARD_HEIGHT); rows]).split(area);
    for (r, row) in row_areas.iter().enumerate() {
        let cells = Layout::horizontal(vec![Constraint::Ratio(1, cols as u32); cols]).split(*row);
        for (c, cell) in cells.iter().enumerate() {
            let i = r * cols + c;
            if i < cities.len() {
                draw_card(f, *cell, &cities[i], &slots[i]);
            }
        }
    }
}

fn draw_card(f: &mut Frame, area: Rect, city: &City, slot: &Slot) {
    let now = Timestamp::now().to_zoned(city.tz.clone());
    let day = slot.weather.as_ref().is_none_or(|w| w.current.is_day == 1);
    let accent = if day { Color::Yellow } else { Color::Blue };

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(accent)
        .title(Line::from(format!(" {} ", city.name)).bold())
        .title(Line::from(format!(" {} ", now.strftime("UTC%:z"))).right_aligned());
    let inner = block.inner(area);
    f.render_widget(block, area);

    let [clock, _, info, _, days] = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(2),
    ])
    .areas(inner);

    let big = BigText::builder()
        .pixel_size(PixelSize::Quadrant)
        .style(Style::new().fg(accent))
        .lines(vec![now.strftime("%H:%M").to_string().into()])
        .centered()
        .build();
    f.render_widget(big, clock);
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
    let cities = names.iter().map(|n| geocode(n)).collect::<Res<Vec<_>>>()?;

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
    let mut terminal = ratatui::init();
    let result = (|| -> Res<()> {
        loop {
            while let Ok((i, r)) = rx.try_recv() {
                match r {
                    Ok(w) => {
                        slots[i] = Slot {
                            weather: Some(w),
                            error: false,
                        }
                    }
                    Err(_) => slots[i].error = true,
                }
            }
            terminal.draw(|f| draw(f, &cities, &slots))?;
            if event::poll(Duration::from_millis(500))?
                && let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
            {
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('r') => _ = refresh_tx.send(()),
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
                      "temperature_2m_max": [3.6, 5.0], "temperature_2m_min": [-2.5, 1.0]}
        }"#;
        let w: Weather = serde_json::from_str(json).unwrap();
        assert_eq!(deg(w.current.temperature_2m), 0);
        assert_eq!(icon(w.current.weather_code).0, "❄");
        assert_eq!(icon(w.daily.weather_code[1]).0, "⚡");
        assert_eq!(icon(2).0, "☁");
        assert_eq!(icon(200).0, "?");
    }

    #[test]
    fn parses_cities_file() {
        let text = "Warsaw\n\n  New York  \n# Lisbon\nTokyo\n";
        assert_eq!(parse_cities(text), ["Warsaw", "New York", "Tokyo"]);
    }
}
