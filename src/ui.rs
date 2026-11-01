use jiff::{Timestamp, civil::Date, tz::TimeZone};
use ratatui::{
    Frame,
    layout::{Constraint, Flex, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph},
};

use crate::weather::{City, Weather};

const CARD_WIDTH: u16 = 38;
const CARD_HEIGHT: u16 = 12;

#[derive(Default)]
pub(crate) struct Slot {
    pub(crate) weather: Option<Weather>,
    pub(crate) error: bool,
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
pub(crate) fn draw(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_icons_and_rounds_degrees() {
        assert_eq!(deg(-0.4), 0);
        assert_eq!(icon(71).0, "❄");
        assert_eq!(icon(95).0, "⚡");
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
}
