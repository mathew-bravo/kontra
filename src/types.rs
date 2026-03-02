use std::collections::{BTreeSet, HashMap};

use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObligationState {
    Pending,
    Active,
    Satisfied,
    Breached,
    Remedied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Party {
    pub role: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventDef {
    DateEvent(NaiveDate),
    TriggeredEvent(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventOccurrence {
    pub name: String,
    pub date: NaiveDate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurationUnit {
    CalendarDays,
    BusinessDays,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duration {
    pub amount: u32,
    pub unit: DurationUnit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BusinessWeekday {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl BusinessWeekday {
    pub fn standard_business_days() -> BTreeSet<Self> {
        [Self::Mon, Self::Tue, Self::Wed, Self::Thu, Self::Fri]
            .into_iter()
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarDef {
    pub id: String,
    pub jurisdiction: Option<String>,
    pub business_weekdays: BTreeSet<BusinessWeekday>,
    pub holidays: BTreeSet<NaiveDate>,
}

impl CalendarDef {
    pub fn standard(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            jurisdiction: None,
            business_weekdays: BusinessWeekday::standard_business_days(),
            holidays: BTreeSet::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarRegistry {
    pub default_calendar_id: String,
    pub calendars: HashMap<String, CalendarDef>,
}

impl CalendarRegistry {
    pub const PHASE2_DEFAULT_CALENDAR_ID: &'static str = "default";

    pub fn phase2_default() -> Self {
        let default_calendar = CalendarDef::standard(Self::PHASE2_DEFAULT_CALENDAR_ID);
        let mut calendars = HashMap::new();
        calendars.insert(default_calendar.id.clone(), default_calendar);
        Self {
            default_calendar_id: Self::PHASE2_DEFAULT_CALENDAR_ID.to_string(),
            calendars,
        }
    }

    pub fn from_calendars(
        default_calendar_id: impl Into<String>,
        calendars: Vec<CalendarDef>,
    ) -> Result<Self, String> {
        if calendars.is_empty() {
            return Ok(Self::phase2_default());
        }

        let mut map = HashMap::new();
        for calendar in calendars {
            if map.contains_key(&calendar.id) {
                return Err(format!("Duplicate calendar id '{}'", calendar.id));
            }
            map.insert(calendar.id.clone(), calendar);
        }

        Ok(Self {
            default_calendar_id: default_calendar_id.into(),
            calendars: map,
        }
        .normalize())
    }

    pub fn normalize(mut self) -> Self {
        self.normalize_in_place();
        self
    }

    pub fn has_calendar(&self, id: &str) -> bool {
        self.calendars.contains_key(id)
    }

    pub fn default_calendar(&self) -> Option<&CalendarDef> {
        self.calendars.get(&self.default_calendar_id)
    }

    fn normalize_in_place(&mut self) {
        if self.calendars.is_empty() {
            *self = Self::phase2_default();
            return;
        }

        if self.default_calendar_id.is_empty()
            || !self.calendars.contains_key(&self.default_calendar_id)
        {
            let mut ids: Vec<String> = self.calendars.keys().cloned().collect();
            ids.sort();
            self.default_calendar_id = ids
                .into_iter()
                .next()
                .expect("calendar map should not be empty after guard");
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermAnchor {
    Event(String),
    BreachOf(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    pub name: String,
    pub duration: Duration,
    pub anchor: TermAnchor,
    pub calendar_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConditionExpr {
    After(String),
    Before(String),
    Satisfied(String),
    Occurred(String),
    And(Box<ConditionExpr>, Box<ConditionExpr>),
    Or(Box<ConditionExpr>, Box<ConditionExpr>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DueDef {
    TermRef(String),
    InlineDuration {
        duration: Duration,
        anchor: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObligationDef {
    pub name: String,
    pub party_role: Option<String>,
    pub action: Option<String>,
    pub due: Option<DueDef>,
    pub condition: Option<ConditionExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseDef {
    pub name: String,
    pub breach_target: Option<String>,
    pub party_role: Option<String>,
    pub action: Option<String>,
    pub due: Option<DueDef>,
    pub condition: Option<ConditionExpr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemedyDef {
    pub name: String,
    pub breach_target: String,
    pub party_role: Option<String>,
    pub action: Option<String>,
    pub due: Option<DueDef>,
    pub condition: Option<ConditionExpr>,
    pub phases: Vec<PhaseDef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractDef {
    pub parties: HashMap<String, Party>,
    pub events: HashMap<String, EventDef>,
    pub terms: HashMap<String, Term>,
    pub obligations: HashMap<String, ObligationDef>,
    pub remedies: HashMap<String, RemedyDef>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_calendars_rejects_duplicate_ids() {
        let calendars = vec![CalendarDef::standard("us"), CalendarDef::standard("us")];
        let result = CalendarRegistry::from_calendars("us", calendars);
        assert!(result.is_err());
        assert!(
            result
                .expect_err("duplicate id should error")
                .contains("Duplicate calendar id 'us'")
        );
    }

    #[test]
    fn from_calendars_empty_falls_back_to_phase2_default() {
        let registry = CalendarRegistry::from_calendars("", Vec::new())
            .expect("empty calendars should normalize to default");
        assert_eq!(
            registry.default_calendar_id,
            CalendarRegistry::PHASE2_DEFAULT_CALENDAR_ID
        );
        assert!(
            registry.has_calendar(CalendarRegistry::PHASE2_DEFAULT_CALENDAR_ID),
            "default calendar should be present",
        );
    }

    #[test]
    fn normalize_uses_sorted_first_calendar_when_default_is_invalid() {
        let mut calendars = HashMap::new();
        calendars.insert("zeta".to_string(), CalendarDef::standard("zeta"));
        calendars.insert("alpha".to_string(), CalendarDef::standard("alpha"));
        let registry = CalendarRegistry {
            default_calendar_id: "missing".to_string(),
            calendars,
        };

        let normalized = registry.normalize();
        assert_eq!(normalized.default_calendar_id, "alpha");
    }

    #[test]
    fn standard_calendar_defaults_to_weekdays_only() {
        let calendar = CalendarDef::standard("default");
        assert!(calendar.business_weekdays.contains(&BusinessWeekday::Mon));
        assert!(calendar.business_weekdays.contains(&BusinessWeekday::Fri));
        assert!(!calendar.business_weekdays.contains(&BusinessWeekday::Sat));
        assert!(!calendar.business_weekdays.contains(&BusinessWeekday::Sun));
        assert!(calendar.holidays.is_empty());
    }
}
