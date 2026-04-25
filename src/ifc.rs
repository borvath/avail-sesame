use std::collections::BTreeSet;

use anyhow::anyhow;
use chrono::{FixedOffset, Local};
use sesame::{
    context::{Context, UnprotectedContext},
    critical::{CriticalRegion, Signature},
    pcon::PCon,
    policy::{AnyPolicyClone, OptionPolicy, Reason, SimplePolicy},
    verified::VerifiedRegion,
};

use crate::{
    datetime::{availability::Availability, finder::AvailabilityFinder},
    events::{Calendar, Event},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CalendarSecrecyPolicy {
    owners: BTreeSet<String>,
}

impl CalendarSecrecyPolicy {
    pub fn for_owner(owner: impl Into<String>) -> Self {
        let mut owners = BTreeSet::new();
        owners.insert(owner.into());
        Self { owners }
    }

    pub fn for_owners(owners: BTreeSet<String>) -> Self {
        Self { owners }
    }

    pub fn owners(&self) -> &BTreeSet<String> {
        &self.owners
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevealRoute {
    RefreshCalendarsPrompt,
    RefreshCalendarsHoldEventTarget,
    AvailabilityCompute,
    AvailabilitySelection,
    AvailabilityOutput,
    HoldEventsSchedule,
    HoldEventsTargetCalendar,
}

impl RevealRoute {
    fn as_str(self) -> &'static str {
        match self {
            Self::RefreshCalendarsPrompt => "refresh_calendars.prompt",
            Self::RefreshCalendarsHoldEventTarget => "refresh_calendars.hold_event_target",
            Self::AvailabilityCompute => "availability.compute",
            Self::AvailabilitySelection => "availability.selection",
            Self::AvailabilityOutput => "availability.output",
            Self::HoldEventsSchedule => "hold_events.schedule",
            Self::HoldEventsTargetCalendar => "hold_events.target_calendar",
        }
    }

    fn allows_release(route: &str) -> bool {
        matches!(
            route,
            "refresh_calendars.prompt"
                | "refresh_calendars.hold_event_target"
                | "availability.compute"
                | "availability.selection"
                | "availability.output"
                | "hold_events.schedule"
                | "hold_events.target_calendar"
        )
    }
}

impl std::fmt::Display for RevealRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl SimplePolicy for CalendarSecrecyPolicy {
    fn simple_name(&self) -> String {
        format!("CalendarSecrecyPolicy({:?})", self.owners)
    }

    fn simple_check(&self, context: &UnprotectedContext, _reason: Reason<'_>) -> bool {
        if !RevealRoute::allows_release(&context.route) {
            return false;
        }

        let Some(authorized_owners) = context.downcast_ref::<Vec<String>>() else {
            return false;
        };

        let authorized_owners: BTreeSet<&str> =
            authorized_owners.iter().map(String::as_str).collect();

        self.owners
            .iter()
            .all(|owner| authorized_owners.contains(owner.as_str()))
    }

    fn simple_join_direct(&mut self, other: &mut Self) {
        self.owners.extend(other.owners.iter().cloned());
    }
}

pub type ProtectedCalendar = PCon<Calendar, CalendarSecrecyPolicy>;
pub type ProtectedEvent = PCon<Event, CalendarSecrecyPolicy>;
pub type ProtectedEvents = PCon<Vec<Event>, CalendarSecrecyPolicy>;
pub type ProtectedAvailability = PCon<Vec<Availability<Local>>, AnyPolicyClone>;

fn localize_availability(
    slots: Vec<Availability<FixedOffset>>,
) -> Vec<Availability<Local>> {
    slots
        .into_iter()
        .map(|slot| Availability {
            start: slot.start.with_timezone(&Local),
            end: slot.end.with_timezone(&Local),
        })
        .collect()
}

pub fn protect_calendar(calendar: Calendar, owner: &str) -> ProtectedCalendar {
    PCon::new(calendar, CalendarSecrecyPolicy::for_owner(owner))
}

pub fn protect_event(event: Event, owner: &str) -> ProtectedEvent {
    PCon::new(event, CalendarSecrecyPolicy::for_owner(owner))
}

pub fn protect_events(events: Vec<Event>, owner: &str) -> ProtectedEvents {
    PCon::new(events, CalendarSecrecyPolicy::for_owner(owner))
}

pub fn reveal<T: Clone, P: sesame::policy::Policy>(
    route: RevealRoute,
    value: &PCon<T, P>,
    authorized_owners: &BTreeSet<String>,
) -> anyhow::Result<T> {
    value
        .critical(
            Context::new(
                route.to_string(),
                authorized_owners.iter().cloned().collect::<Vec<_>>(),
            ),
            CriticalRegion::new(
                |data: &T, _| data.clone(),
                Signature {
                    username: "avail",
                    signature: "sesame-ifc-eval",
                },
            ),
            (),
        )
        .map_err(|_| anyhow!("Sesame blocked externalization for {}", route))
}

fn owners_from_event_batch_policy(
    policy: &OptionPolicy<CalendarSecrecyPolicy>,
) -> BTreeSet<String> {
    match policy {
        OptionPolicy::NoPolicy => BTreeSet::new(),
        OptionPolicy::Policy(policy) => policy.owners().clone(),
    }
}

pub fn compute_availability(
    finder: &AvailabilityFinder,
    event_batches: Vec<ProtectedEvents>,
) -> anyhow::Result<ProtectedAvailability> {
    let protected_event_batches: PCon<Vec<Vec<Event>>, _> = event_batches.into();
    let authorized_owners = owners_from_event_batch_policy(protected_event_batches.policy());
    let availability_owners = authorized_owners.clone();

    protected_event_batches
        .into_verified(VerifiedRegion::new(|event_batches: Vec<Vec<Event>>| {
            event_batches.into_iter().flatten().collect::<Vec<_>>()
        }))
        .into_verified(VerifiedRegion::new(|events| {
            finder
                .get_availability_fixed(events)
                .into_iter()
                .flat_map(|(_day, slots)| slots)
                .collect::<Vec<_>>()
        }))
        .into_critical(
            Context::new(
                RevealRoute::AvailabilityCompute.to_string(),
                authorized_owners.iter().cloned().collect::<Vec<_>>(),
            ),
            CriticalRegion::new(
                |slots, _| {
                    PCon::new(
                        localize_availability(slots),
                        AnyPolicyClone::new(CalendarSecrecyPolicy::for_owners(availability_owners)),
                    )
                },
                Signature {
                    username: "avail",
                    signature: "sesame-ifc-eval",
                },
            ),
            (),
        )
        .map_err(|_| anyhow!("Sesame blocked policy-preserving availability.compute"))
}

pub fn owners_from_availability(
    availability: &ProtectedAvailability,
) -> anyhow::Result<BTreeSet<String>> {
    availability
        .policy()
        .specialize_top_ref::<CalendarSecrecyPolicy>()
        .map(|policy| policy.owners().clone())
        .map_err(|err| anyhow!("failed to recover availability policy: {}", err))
}

pub fn rewrap_availability(
    slots: Vec<Availability<Local>>,
    availability: &ProtectedAvailability,
) -> ProtectedAvailability {
    PCon::new(slots, availability.policy().clone())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chrono::{DateTime, Local};

    use super::*;

    fn dt(value: &str) -> DateTime<Local> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Local)
    }

    #[test]
    fn calendar_policy_requires_the_owner() {
        let protected = protect_event(
            Event {
                id: String::from("event-1"),
                name: Some(String::from("secret")),
                start: dt("2026-04-16T09:00:00-04:00"),
                end: dt("2026-04-16T10:00:00-04:00"),
            },
            "alice@example.com",
        );

        let allowed = reveal(
            RevealRoute::AvailabilityOutput,
            &protected,
            &BTreeSet::from([String::from("alice@example.com")]),
        );
        assert!(allowed.is_ok());

        let denied = reveal(
            RevealRoute::AvailabilityOutput,
            &protected,
            &BTreeSet::from([String::from("bob@example.com")]),
        );
        assert!(denied.is_err());
    }

    #[test]
    fn joined_calendar_policy_accumulates_all_owners() {
        let authorized_owners = BTreeSet::from([
            String::from("alice@example.com"),
            String::from("bob@example.com"),
        ]);
        let protected_availability = compute_availability(
            &AvailabilityFinder {
                start: dt("2026-04-16T09:00:00-04:00"),
                end: dt("2026-04-16T17:00:00-04:00"),
                min: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                max: chrono::NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
                duration: chrono::Duration::minutes(30),
                include_weekends: true,
            },
            vec![
                protect_events(vec![], "alice@example.com"),
                protect_events(vec![], "bob@example.com"),
            ],
        )
        .unwrap();

        assert_eq!(
            owners_from_availability(&protected_availability).unwrap(),
            authorized_owners
        );
    }
}
