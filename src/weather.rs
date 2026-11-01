use jiff::tz::TimeZone;
use serde::Deserialize;

use crate::Res;

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
pub(crate) struct Current {
    pub(crate) temperature_2m: f64,
    pub(crate) weather_code: u8,
    pub(crate) is_day: u8,
}

#[derive(Deserialize)]
pub(crate) struct Daily {
    pub(crate) time: Vec<String>,
    pub(crate) weather_code: Vec<u8>,
    pub(crate) temperature_2m_max: Vec<f64>,
    pub(crate) temperature_2m_min: Vec<f64>,
    pub(crate) sunrise: Vec<String>,
    pub(crate) sunset: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct Weather {
    pub(crate) current: Current,
    pub(crate) daily: Daily,
}

pub(crate) struct City {
    pub(crate) name: String,
    pub(crate) country: String,
    pub(crate) tz: TimeZone,
    pub(crate) coords: (f64, f64),
}

fn get(url: &str, query: &[(&str, &str)]) -> Res<String> {
    let mut req = ureq::get(url);
    for (k, v) in query {
        req = req.query(*k, *v);
    }
    Ok(req.call()?.body_mut().read_to_string()?)
}

pub(crate) fn geocode(name: &str) -> Res<City> {
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

pub(crate) fn fetch_weather((lat, lon): (f64, f64)) -> Res<Weather> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_forecast() {
        let json = r#"{
            "current": {"temperature_2m": -0.4, "weather_code": 71, "is_day": 0},
            "daily": {"time": ["2026-11-01", "2026-11-02"], "weather_code": [0, 95],
                      "temperature_2m_max": [3.6, 5.0], "temperature_2m_min": [-2.5, 1.0],
                      "sunrise": ["2026-11-01T06:41", "2026-11-02T06:43"],
                      "sunset": ["2026-11-01T16:23", "2026-11-02T16:21"]}
        }"#;
        let w: Weather = serde_json::from_str(json).unwrap();
        assert_eq!(w.current.temperature_2m, -0.4);
        assert_eq!(w.current.weather_code, 71);
        assert_eq!(w.daily.weather_code, [0, 95]);
        assert_eq!(w.daily.sunset[1], "2026-11-02T16:21");
    }

    #[test]
    fn place_without_country_still_parses() {
        let geo: Geo = serde_json::from_str(
            r#"{"results": [{"name": "X", "latitude": 1.0, "longitude": 2.0, "timezone": "UTC"}]}"#,
        )
        .unwrap();
        assert_eq!(geo.results.unwrap()[0].country, "");
    }
}
