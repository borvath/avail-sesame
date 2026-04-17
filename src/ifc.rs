use std::collections::BTreeSet;

use anyhow::anyhow;
use chrono::Local;
use sesame::{
    context::{Context, UnprotectedContext},
    critical::{CriticalRegion, Signature},
    pcon::PCon,
    policy::{AnyPolicyClone, Reason, SimplePolicy},
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

    pub fn owners(&self) -> &BTreeSet<String> {
        &self.owners
    }
}

impl SimplePolicy for CalendarSecrecyPolicy {
    fn simple_name(&self) -> String {
        format!("CalendarSecrecyPolicy({:?})", self.owners)
    }

    fn simple_check(&self, context: &UnprotectedContext, _reason: Reason<'_>) -> bool {
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
pub type ProtectedAvailability = PCon<Vec<Availability<Local>>, AnyPolicyClone>;

pub fn protect_calendar(calendar: Calendar, owner: &str) -> ProtectedCalendar {
    PCon::new(calendar, CalendarSecrecyPolicy::for_owner(owner))
}

pub fn protect_event(event: Event, owner: &str) -> ProtectedEvent {
    PCon::new(event, CalendarSecrecyPolicy::for_owner(owner))
}

pub fn reveal<T: Clone, P: sesame::policy::Policy>(
    route: &str,
    value: &PCon<T, P>,
    authorized_owners: &BTreeSet<String>,
) -> anyhow::Result<T> {
    value
        .critical(
            Context::new(
                route.to_owned(),
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

pub fn compute_availability(
    finder: &AvailabilityFinder,
    authorized_owners: &BTreeSet<String>,
    events: Vec<ProtectedEvent>,
) -> anyhow::Result<ProtectedAvailability> {
    let protected_events: PCon<Vec<Event>, _> = events.into();
    let events = reveal("availability.compute", &protected_events, authorized_owners)?;
    let slots = finder
        .get_availability(events)?
        .into_iter()
        .flat_map(|(_day, slots)| slots)
        .collect::<Vec<_>>();

    Ok(PCon::new(
        slots,
        AnyPolicyClone::new(CalendarSecrecyPolicy {
            owners: authorized_owners.clone(),
        }),
    ))
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
            "test.allowed",
            &protected,
            &BTreeSet::from([String::from("alice@example.com")]),
        );
        assert!(allowed.is_ok());

        let denied = reveal(
            "test.denied",
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
            &authorized_owners,
            vec![],
        )
        .unwrap();

        assert_eq!(
            owners_from_availability(&protected_availability).unwrap(),
            authorized_owners
        );
    }
}
