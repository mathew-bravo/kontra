use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::Deserialize;

use crate::types::{BusinessWeekday, CalendarDef, CalendarRegistry};

const ENV_CALENDAR_CONFIG_PATH: &str = "KONTRA_CALENDAR_CONFIG";
const DEFAULT_CALENDAR_CONFIG_PATH: &str = "kontra-calendars.json";

#[derive(Debug, Deserialize)]
struct CalendarRegistryFile {
    default_calendar_id: Option<String>,
    #[serde(default)]
    calendars: Vec<CalendarFileEntry>,
}

#[derive(Debug, Deserialize)]
struct CalendarFileEntry {
    id: String,
    jurisdiction: Option<String>,
    business_weekdays: Option<Vec<String>>,
    holidays: Option<Vec<String>>,
}

pub fn load_calendar_registry() -> Result<CalendarRegistry, String> {
    let Some(config_path) = resolve_calendar_config_path() else {
        return Ok(CalendarRegistry::phase2_default());
    };
    load_calendar_registry_from_path(config_path.as_path())
}

pub fn load_calendar_registry_from_path(path: &Path) -> Result<CalendarRegistry, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("Failed to read calendar config '{}': {}", path.display(), err))?;

    parse_calendar_registry_json(&raw).map_err(|err| {
        format!(
            "Invalid calendar config '{}': {}",
            path.display(),
            err
        )
    })
}

fn resolve_calendar_config_path() -> Option<PathBuf> {
    if let Some(raw_path) = env::var_os(ENV_CALENDAR_CONFIG_PATH) {
        let candidate = PathBuf::from(raw_path);
        if candidate.exists() {
            return Some(candidate);
        }
        return None;
    }

    let default_path = PathBuf::from(DEFAULT_CALENDAR_CONFIG_PATH);
    if default_path.exists() {
        return Some(default_path);
    }

    None
}

fn parse_calendar_registry_json(raw: &str) -> Result<CalendarRegistry, String> {
    let parsed: CalendarRegistryFile =
        serde_json::from_str(raw).map_err(|err| format!("JSON parse failed: {}", err))?;

    let default_calendar_id = parsed.default_calendar_id.unwrap_or_default();
    let mut calendars = Vec::with_capacity(parsed.calendars.len());

    for calendar in parsed.calendars {
        if calendar.id.trim().is_empty() {
            return Err("calendar id cannot be empty".to_string());
        }

        let business_weekdays = match calendar.business_weekdays {
            Some(raw_weekdays) => parse_business_weekdays(&calendar.id, raw_weekdays)?,
            None => BusinessWeekday::standard_business_days(),
        };

        let holidays = parse_holidays(&calendar.id, calendar.holidays.unwrap_or_default())?;

        calendars.push(CalendarDef {
            id: calendar.id,
            jurisdiction: calendar.jurisdiction,
            business_weekdays,
            holidays,
        });
    }

    CalendarRegistry::from_calendars(default_calendar_id, calendars)
}

fn parse_business_weekdays(
    calendar_id: &str,
    weekdays: Vec<String>,
) -> Result<BTreeSet<BusinessWeekday>, String> {
    let mut parsed = BTreeSet::new();
    for raw in weekdays {
        let weekday = parse_business_weekday(&raw).ok_or_else(|| {
            format!(
                "calendar '{}' has invalid weekday '{}'",
                calendar_id, raw
            )
        })?;
        parsed.insert(weekday);
    }

    if parsed.is_empty() {
        return Err(format!(
            "calendar '{}' must declare at least one business weekday",
            calendar_id
        ));
    }

    Ok(parsed)
}

fn parse_business_weekday(raw: &str) -> Option<BusinessWeekday> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(BusinessWeekday::Mon),
        "tue" | "tues" | "tuesday" => Some(BusinessWeekday::Tue),
        "wed" | "wednesday" => Some(BusinessWeekday::Wed),
        "thu" | "thurs" | "thursday" => Some(BusinessWeekday::Thu),
        "fri" | "friday" => Some(BusinessWeekday::Fri),
        "sat" | "saturday" => Some(BusinessWeekday::Sat),
        "sun" | "sunday" => Some(BusinessWeekday::Sun),
        _ => None,
    }
}

fn parse_holidays(
    calendar_id: &str,
    holidays: Vec<String>,
) -> Result<BTreeSet<NaiveDate>, String> {
    let mut parsed = BTreeSet::new();
    for raw in holidays {
        let date = NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").map_err(|_| {
            format!(
                "calendar '{}' has invalid holiday '{}'; expected YYYY-MM-DD",
                calendar_id, raw
            )
        })?;
        parsed.insert(date);
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn parse_registry_json_supports_custom_default_and_calendar_fields() {
        let raw = r#"{
            "default_calendar_id": "us_ny",
            "calendars": [
                {
                    "id": "us_ny",
                    "jurisdiction": "US-NY",
                    "business_weekdays": ["Mon", "Tue", "Wed", "Thu", "Fri"],
                    "holidays": ["2026-01-01", "2026-12-25"]
                }
            ]
        }"#;

        let registry = parse_calendar_registry_json(raw).expect("config should parse");
        assert_eq!(registry.default_calendar_id, "us_ny");

        let calendar = registry
            .calendars
            .get("us_ny")
            .expect("calendar should exist");
        assert_eq!(calendar.jurisdiction.as_deref(), Some("US-NY"));
        assert!(calendar.business_weekdays.contains(&BusinessWeekday::Mon));
        assert!(!calendar.business_weekdays.contains(&BusinessWeekday::Sat));
        assert!(
            calendar
                .holidays
                .contains(&NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date"))
        );
    }

    #[test]
    fn parse_registry_json_normalizes_invalid_default_id() {
        let raw = r#"{
            "default_calendar_id": "missing",
            "calendars": [
                { "id": "zeta" },
                { "id": "alpha" }
            ]
        }"#;

        let registry = parse_calendar_registry_json(raw).expect("config should parse");
        assert_eq!(registry.default_calendar_id, "alpha");
    }

    #[test]
    fn parse_registry_json_empty_calendars_falls_back_to_phase2_default() {
        let raw = r#"{
            "default_calendar_id": "does-not-matter",
            "calendars": []
        }"#;

        let registry = parse_calendar_registry_json(raw).expect("config should parse");
        assert_eq!(
            registry.default_calendar_id,
            CalendarRegistry::PHASE2_DEFAULT_CALENDAR_ID
        );
        assert!(
            registry.has_calendar(CalendarRegistry::PHASE2_DEFAULT_CALENDAR_ID),
            "phase2 default calendar should be present",
        );
    }

    #[test]
    fn parse_registry_json_rejects_invalid_weekday() {
        let raw = r#"{
            "calendars": [
                {
                    "id": "default",
                    "business_weekdays": ["Mon", "Freeday"]
                }
            ]
        }"#;

        let err = parse_calendar_registry_json(raw).expect_err("invalid weekday should fail");
        assert!(err.contains("invalid weekday 'Freeday'"));
    }

    #[test]
    fn parse_registry_json_rejects_invalid_holiday_format() {
        let raw = r#"{
            "calendars": [
                {
                    "id": "default",
                    "holidays": ["01/01/2026"]
                }
            ]
        }"#;

        let err = parse_calendar_registry_json(raw).expect_err("invalid holiday should fail");
        assert!(err.contains("expected YYYY-MM-DD"));
    }

    #[test]
    fn load_calendar_registry_from_path_reads_json_file() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        let file_name = format!("kontra-calendar-config-{}.json", nonce);
        let path = env::temp_dir().join(file_name);
        let raw = r#"{
            "default_calendar_id": "custom",
            "calendars": [{ "id": "custom" }]
        }"#;

        fs::write(&path, raw).expect("should write temp config");
        let registry =
            load_calendar_registry_from_path(path.as_path()).expect("loader should parse file");
        assert_eq!(registry.default_calendar_id, "custom");
        assert!(registry.has_calendar("custom"));

        fs::remove_file(path).expect("should cleanup temp config");
    }
}
