use chrono::{Datelike, Duration as ChronoDuration, NaiveDate, Weekday};

use crate::types::{BusinessWeekday, CalendarDef};

pub fn is_business_day(date: NaiveDate, calendar: &CalendarDef) -> bool {
    let weekday = from_chrono_weekday(date.weekday());
    calendar.business_weekdays.contains(&weekday) && !calendar.holidays.contains(&date)
}

pub fn add_business_days(date: NaiveDate, days: u32, calendar: &CalendarDef) -> NaiveDate {
    if days == 0 {
        return date;
    }

    let mut remaining = days;
    let mut cursor = date;

    while remaining > 0 {
        cursor += ChronoDuration::days(1);
        if is_business_day(cursor, calendar) {
            remaining -= 1;
        }
    }

    cursor
}

pub fn next_business_day(date: NaiveDate, calendar: &CalendarDef) -> NaiveDate {
    let mut cursor = date;
    loop {
        cursor += ChronoDuration::days(1);
        if is_business_day(cursor, calendar) {
            return cursor;
        }
    }
}

fn from_chrono_weekday(weekday: Weekday) -> BusinessWeekday {
    match weekday {
        Weekday::Mon => BusinessWeekday::Mon,
        Weekday::Tue => BusinessWeekday::Tue,
        Weekday::Wed => BusinessWeekday::Wed,
        Weekday::Thu => BusinessWeekday::Thu,
        Weekday::Fri => BusinessWeekday::Fri,
        Weekday::Sat => BusinessWeekday::Sat,
        Weekday::Sun => BusinessWeekday::Sun,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn is_business_day_respects_weekend_and_holiday_rules() {
        let mut calendar = CalendarDef::standard("default");
        let holiday = NaiveDate::from_ymd_opt(2026, 3, 9).expect("valid date");
        calendar.holidays.insert(holiday);

        assert!(
            is_business_day(
                NaiveDate::from_ymd_opt(2026, 3, 10).expect("valid date"),
                &calendar
            ),
            "Tuesday should be business day",
        );
        assert!(
            !is_business_day(
                NaiveDate::from_ymd_opt(2026, 3, 8).expect("valid date"),
                &calendar
            ),
            "Sunday should not be business day",
        );
        assert!(
            !is_business_day(holiday, &calendar),
            "holiday should not be business day",
        );
    }

    #[test]
    fn add_business_days_skips_weekends_and_crosses_month_and_year() {
        let calendar = CalendarDef::standard("default");
        let start = NaiveDate::from_ymd_opt(2026, 12, 31).expect("valid date"); // Thu
        let due = add_business_days(start, 2, &calendar);
        assert_eq!(
            due,
            NaiveDate::from_ymd_opt(2027, 1, 4).expect("valid date"),
            "2 business days from Thu 2026-12-31 should land on Mon 2027-01-04",
        );
    }

    #[test]
    fn add_business_days_skips_configured_holidays() {
        let mut holidays = BTreeSet::new();
        holidays.insert(NaiveDate::from_ymd_opt(2026, 3, 9).expect("valid date")); // Monday
        let calendar = CalendarDef {
            id: "us_ny".to_string(),
            jurisdiction: Some("US-NY".to_string()),
            business_weekdays: BusinessWeekday::standard_business_days(),
            holidays,
        };

        let start = NaiveDate::from_ymd_opt(2026, 3, 6).expect("valid date"); // Fri
        let due = add_business_days(start, 1, &calendar);
        assert_eq!(
            due,
            NaiveDate::from_ymd_opt(2026, 3, 10).expect("valid date"),
            "holiday Monday should push +1 business day to Tuesday",
        );
    }

    #[test]
    fn next_business_day_moves_to_next_valid_day() {
        let calendar = CalendarDef::standard("default");
        let next = next_business_day(
            NaiveDate::from_ymd_opt(2026, 3, 6).expect("valid date"), // Fri
            &calendar,
        );
        assert_eq!(next, NaiveDate::from_ymd_opt(2026, 3, 9).expect("valid date"));
    }
}
