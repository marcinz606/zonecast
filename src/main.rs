mod ui;
mod weather;

use std::{sync::mpsc, thread, time::Duration};

use jiff::Timestamp;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};

use ui::Slot;
use weather::{City, fetch_weather, geocode};

type Res<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

const REFRESH: Duration = Duration::from_secs(15 * 60);

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
            terminal.draw(|f| ui::draw(f, &cities, &slots, updated, &mut scroll))?;
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
    use jiff::tz::TimeZone;

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
    fn parses_cities_file() {
        let text = "Warsaw\n\n  New York  \n# Lisbon\nTokyo\n";
        assert_eq!(parse_cities(text), ["Warsaw", "New York", "Tokyo"]);
    }
}
