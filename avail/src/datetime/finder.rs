use chrono::{offset::Offset, prelude::*, Duration, FixedOffset};

use crate::events::Event;

use super::availability::Availability;

pub struct AvailabilityFinder {
    pub start: DateTime<Local>,
    pub end: DateTime<Local>,
    pub min: NaiveTime,
    pub max: NaiveTime,
    pub duration: Duration,
    pub include_weekends: bool,
}

fn is_weekend(weekday: Weekday) -> bool {
    weekday == Weekday::Sat || weekday == Weekday::Sun
}

#[derive(Clone)]
struct TimedEvent<T: TimeZone>
where
    <T as TimeZone>::Offset: Copy,
{
    start: DateTime<T>,
    end: DateTime<T>,
}

#[allow(clippy::type_complexity)]
impl AvailabilityFinder {
    pub fn get_availability(
        &self,
        events: Vec<Event>,
    ) -> Vec<(Date<Local>, Vec<Availability<Local>>)> {
        get_availability_impl(
            self.start,
            self.end,
            self.min,
            self.max,
            self.duration,
            self.include_weekends,
            local_events(events),
        )
    }

    pub(crate) fn get_availability_fixed(
        &self,
        events: Vec<Event>,
    ) -> Vec<(Date<FixedOffset>, Vec<Availability<FixedOffset>>)> {
        get_availability_impl(
            self.start.with_timezone(&self.start.offset().fix()),
            self.end.with_timezone(&self.end.offset().fix()),
            self.min,
            self.max,
            self.duration,
            self.include_weekends,
            fixed_events(events),
        )
    }
}

fn local_events(events: Vec<Event>) -> Vec<TimedEvent<Local>> {
    let mut timed_events = Vec::with_capacity(events.len());

    for event in events {
        timed_events.push(TimedEvent {
            start: event.start,
            end: event.end,
        });
    }

    timed_events
}

fn fixed_events(events: Vec<Event>) -> Vec<TimedEvent<FixedOffset>> {
    let mut timed_events = Vec::with_capacity(events.len());

    for event in events {
        timed_events.push(TimedEvent {
            start: event.start.with_timezone(&event.start.offset().fix()),
            end: event.end.with_timezone(&event.end.offset().fix()),
        });
    }

    timed_events
}

fn get_availability_impl<T: TimeZone>(
    start: DateTime<T>,
    end: DateTime<T>,
    min: NaiveTime,
    max: NaiveTime,
    duration: Duration,
    include_weekends: bool,
    events: Vec<TimedEvent<T>>,
) -> Vec<(Date<T>, Vec<Availability<T>>)>
where
    <T as TimeZone>::Offset: Copy,
{
    let mut avail: Vec<(Date<T>, Vec<Availability<T>>)> = vec![];
    let days = group_events_by_date(events);
    let mut iter = days.into_iter();

    let mut curr = start.date().and_hms(min.hour(), min.minute(), 0);
    curr = DateTime::max(curr, start);
    curr = curr.ceil();

    while curr < end {
        let day = iter.next();

        if let Some((date, events)) = day {
            while curr.date() < date {
                if curr.time() < max {
                    let day_end = curr.date().and_hms(max.hour(), max.minute(), 0);

                    if include_weekends || !is_weekend(curr.weekday()) {
                        avail.push((
                            curr.date(),
                            vec![Availability {
                                start: curr.date().and_hms(min.hour(), max.minute(), 0),
                                end: day_end,
                            }],
                        ));
                    }
                }

                curr = (curr + Duration::days(1))
                    .date()
                    .and_hms(min.hour(), min.minute(), 0);
            }

            if !include_weekends && is_weekend(date.weekday()) {
                if curr.date() == date {
                    curr = (curr + Duration::days(1))
                        .date()
                        .and_hms(min.hour(), min.minute(), 0);
                }

                continue;
            }

            let mut day_avail = vec![];
            let mut curr_time = min;

            for event in events {
                let event_start = event.start;
                let event_end = event.end;

                if curr_time < event_start.time() {
                    let avail_start = event_start
                        .date()
                        .and_hms(curr_time.hour(), curr_time.minute(), 0)
                        .ceil();
                    let avail_end =
                        DateTime::min(event_start, curr.date().and_hms(max.hour(), max.minute(), 0))
                            .floor();

                    if avail_end.time() - avail_start.time() >= duration
                        && avail_start.time() < max
                    {
                        day_avail.push(Availability {
                            start: avail_start,
                            end: avail_end,
                        });
                    }
                }

                curr_time = NaiveTime::max(event_end.time(), curr_time);
            }

            if curr_time < max {
                let avail_start = curr
                    .date()
                    .and_hms(curr_time.hour(), curr_time.minute(), 0)
                    .ceil();
                let avail_end = curr.date().and_hms(max.hour(), max.minute(), 0);

                if avail_end - avail_start >= duration {
                    day_avail.push(Availability {
                        start: avail_start,
                        end: avail_end,
                    });
                }
            }

            avail.push((curr.date(), day_avail));
            curr = (curr + Duration::days(1))
                .date()
                .and_hms(min.hour(), min.minute(), 0);
        } else {
            while curr.date() < end.date() || (curr.date() == end.date() && curr < end) {
                if !is_weekend(curr.weekday()) || include_weekends {
                    let slot_start = curr.ceil();
                    let slot_end = curr + (max - slot_start.time());

                    if slot_start.time() <= max && slot_end - slot_start >= duration {
                        avail.push((curr.date(), vec![Availability {
                            start: slot_start,
                            end: slot_end,
                        }]));
                    }
                }

                curr = (curr + Duration::days(1))
                    .date()
                    .and_hms(min.hour(), min.minute(), 0);
            }
        }
    }

    avail
}

fn group_events_by_date<T: TimeZone>(events: Vec<TimedEvent<T>>) -> Vec<(Date<T>, Vec<TimedEvent<T>>)>
where
    <T as TimeZone>::Offset: Copy,
{
    let mut days: Vec<(Date<T>, Vec<TimedEvent<T>>)> = Vec::new();

    for event in events {
        insert_event_by_date(&mut days, event);
    }

    days
}

fn insert_event_by_date<T: TimeZone>(
    days: &mut Vec<(Date<T>, Vec<TimedEvent<T>>)>,
    event: TimedEvent<T>,
)
where
    <T as TimeZone>::Offset: Copy,
{
    let date = event.start.date();
    let mut day_idx = 0;

    while day_idx < days.len() && days[day_idx].0 < date {
        day_idx += 1;
    }

    if day_idx < days.len() && days[day_idx].0 == date {
        insert_event_by_start(&mut days[day_idx].1, event);
    } else {
        days.insert(day_idx, (date, vec![event]));
    }
}

fn insert_event_by_start<T: TimeZone>(events: &mut Vec<TimedEvent<T>>, event: TimedEvent<T>)
where
    <T as TimeZone>::Offset: Copy,
{
    let event_start = event.start;
    let mut event_idx = 0;

    while event_idx < events.len() && events[event_idx].start <= event_start {
        event_idx += 1;
    }

    events.insert(event_idx, event);
}

pub trait Round {
    fn ceil(&self) -> Self;
    fn floor(&self) -> Self;
}

impl<T: TimeZone> Round for DateTime<T> {
    fn ceil(&self) -> Self {
        let time = self.date().and_hms(self.hour(), self.minute(), 0);
        let minute = self.minute();

        let round_to_minute = 30;

        if minute % round_to_minute == 0 {
            return time;
        }

        let new_minute = (minute / round_to_minute + 1) * round_to_minute;

        time + Duration::minutes((new_minute - minute).into())
    }

    fn floor(&self) -> Self {
        let time = self.date().and_hms(self.hour(), self.minute(), 0);

        let round_to_minute: i64 = 30;

        let minute: i64 = self.minute().into();

        if minute % round_to_minute == 0 {
            return time;
        }

        let new_minute = (minute / round_to_minute) * round_to_minute;

        let delta: i64 = new_minute - minute;

        time + Duration::minutes(delta)
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;

    use super::*;

    fn create_local_datetime(dt_str: &str) -> DateTime<Local> {
        let datetime_fmt = "%m-%d-%Y %H:%M";
        let ndt = NaiveDateTime::parse_from_str(dt_str, datetime_fmt).unwrap();
        Local.from_local_datetime(&ndt).unwrap()
    }

    #[test]
    fn test_round_datetime_up() {
        let dt = create_local_datetime("10-05-2022 00:00");
        assert_eq!(dt, dt.ceil());

        let dt = create_local_datetime("10-05-2022 00:02");
        assert_eq!(create_local_datetime("10-05-2022 00:30"), dt.ceil());

        let dt = create_local_datetime("10-05-2022 00:42");
        assert_eq!(create_local_datetime("10-05-2022 01:00"), dt.ceil());

        // Next day
        let dt = create_local_datetime("10-05-2022 23:42");
        assert_eq!(create_local_datetime("10-06-2022 00:00"), dt.ceil());

        // Should disregard seconds
        let dt = create_local_datetime("10-05-2022 00:02") + Duration::seconds(30);
        assert_eq!(create_local_datetime("10-05-2022 00:30"), dt.ceil());
    }

    #[test]
    fn test_round_datetime_down() {
        let dt = create_local_datetime("10-05-2022 00:00");
        assert_eq!(dt, dt.floor());

        let dt2 = create_local_datetime("10-05-2022 00:02");
        assert_eq!(dt, dt2.floor());

        let dt3 = create_local_datetime("10-05-2022 00:42");
        assert_eq!(create_local_datetime("10-05-2022 00:30"), dt3.floor());

        // Should disregard seconds
        let dt4 = create_local_datetime("10-05-2022 00:02") + Duration::seconds(30);
        assert_eq!(dt, dt4.floor());
    }

    fn create_event(start: &str, end: &str) -> Event {
        let event_id = "id";
        let event_name = "name";
        Event {
            id: event_id.to_string(),
            name: Some(event_name.to_string()),
            // 12 PM
            start: create_local_datetime(start),
            // 2 PM
            end: create_local_datetime(end),
        }
    }

    #[test]
    fn test_get_availability() {
        let events = vec![
            // 12pm - 2pm
            create_event("10-05-2022 12:00", "10-05-2022 14:00"),
            // 3:30pm - 4pm
            create_event("10-05-2022 15:30", "10-05-2022 16:00"),
            // 4pm - 6pm
            create_event("10-05-2022 16:00", "10-05-2022 18:00"),
            // 7pm - 9pm (outside min-max window)
            create_event("10-05-2022 19:00", "10-05-2022 21:00"),
            // Next day, 5:30am to 7am (outside min-max window)
            create_event("10-06-2022 05:30", "10-06-2022 07:00"),
            // Next day, 8:30am to 12pm
            create_event("10-06-2022 08:30", "10-06-2022 12:00"),
        ];

        let finder = AvailabilityFinder {
            start: create_local_datetime("10-05-2022 00:00"),
            end: create_local_datetime("10-07-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            include_weekends: true,
        };
        let avails = finder.get_availability(events);

        assert_eq!(avails.len(), 2);
        let mut day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 2);

        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 09:00"),
                end: create_local_datetime("10-05-2022 12:00"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 14:00"),
                end: create_local_datetime("10-05-2022 15:30"),
            }
        );

        day_avails = &avails.get(1).unwrap().1;
        assert_eq!(day_avails.len(), 1);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 12:00"),
                end: create_local_datetime("10-06-2022 17:00"),
            }
        );
    }

    #[test]
    fn test_get_availability_without_weekends() {
        let events = vec![
            // 12pm - 2pm, Friday
            create_event("11-18-2022 12:00", "11-18-2022 14:00"),
            // 3:30pm - 5pm, Friday
            create_event("11-18-2022 15:30", "11-18-2022 17:00"),
            // 3pm - 5pm, Saturday
            create_event("11-19-2022 15:00", "11-19-2022 17:00"),
            // Monday, 8:30am to 11am
            create_event("11-21-2022 08:30", "11-21-2022 11:00"),
            // Monday, 1pm to 2pm
            create_event("11-21-2022 13:00", "11-21-2022 14:00"),
        ];

        let finder = AvailabilityFinder {
            start: create_local_datetime("11-18-2022 00:00"),
            end: create_local_datetime("11-22-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            include_weekends: false,
        };
        let avails = finder.get_availability(events);

        assert_eq!(avails.len(), 2);
        let mut day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 2);

        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("11-18-2022 09:00"),
                end: create_local_datetime("11-18-2022 12:00"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("11-18-2022 14:00"),
                end: create_local_datetime("11-18-2022 15:30"),
            }
        );

        day_avails = &avails.get(1).unwrap().1;
        assert_eq!(day_avails.len(), 2);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("11-21-2022 11:00"),
                end: create_local_datetime("11-21-2022 13:00"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("11-21-2022 14:00"),
                end: create_local_datetime("11-21-2022 17:00"),
            }
        );
    }

    #[test]
    fn test_get_availability_rounding() {
        let events = vec![
            // 11:55am - 12:35pm
            create_event("10-05-2022 11:55", "10-05-2022 12:35"),
            // 1:35pm - 2:10pm
            create_event("10-05-2022 13:35", "10-05-2022 14:10"),
            // 3:30pm - 4:05pm
            create_event("10-05-2022 15:30", "10-05-2022 16:05"),
        ];
        let finder = AvailabilityFinder {
            start: create_local_datetime("10-05-2022 00:00"),
            end: create_local_datetime("10-06-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            include_weekends: true,
        };
        let avails = finder.get_availability(events);

        assert_eq!(avails.len(), 1);
        let day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 4);

        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 09:00"),
                end: create_local_datetime("10-05-2022 11:30"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 13:00"),
                end: create_local_datetime("10-05-2022 13:30"),
            }
        );
        assert_eq!(
            *day_avails.get(2).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 14:30"),
                end: create_local_datetime("10-05-2022 15:30"),
            }
        );
        assert_eq!(
            *day_avails.get(3).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 16:30"),
                end: create_local_datetime("10-05-2022 17:00"),
            }
        );
    }

    #[test]
    fn test_get_availability_no_events() {
        let finder = AvailabilityFinder {
            start: create_local_datetime("10-05-2022 00:00"),
            end: create_local_datetime("10-07-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            include_weekends: true,
        };
        let avails = finder.get_availability(vec![]);

        assert_eq!(avails.len(), 2);
        let mut day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 1);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-05-2022 09:00"),
                end: create_local_datetime("10-05-2022 17:00"),
            }
        );

        day_avails = &avails.get(1).unwrap().1;
        assert_eq!(day_avails.len(), 1);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 09:00"),
                end: create_local_datetime("10-06-2022 17:00"),
            }
        );
    }

    #[test]
    fn test_get_availability_start_with_full_day() {
        let events = vec![
            // No events on start day

            // 12pm - 2pm
            create_event("10-06-2022 12:00", "10-06-2022 14:00"),
            // 3:30pm - 4pm
            create_event("10-06-2022 15:30", "10-06-2022 16:00"),
        ];
        let finder = AvailabilityFinder {
            start: create_local_datetime("10-05-2022 00:00"),
            end: create_local_datetime("10-07-2022 00:00"),
            min: NaiveTime::from_hms(9, 0, 0),
            max: NaiveTime::from_hms(17, 0, 0),
            duration: Duration::minutes(30),
            include_weekends: true,
        };
        let avails = finder.get_availability(events);

        assert_eq!(avails.len(), 2);
        let mut day_avails = &avails.get(0).unwrap().1;
        assert_eq!(day_avails.len(), 1);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            // Full day
            Availability {
                start: create_local_datetime("10-05-2022 09:00"),
                end: create_local_datetime("10-05-2022 17:00"),
            }
        );

        day_avails = &avails.get(1).unwrap().1;
        assert_eq!(day_avails.len(), 3);
        assert_eq!(
            *day_avails.get(0).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 09:00"),
                end: create_local_datetime("10-06-2022 12:00"),
            }
        );
        assert_eq!(
            *day_avails.get(1).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 14:00"),
                end: create_local_datetime("10-06-2022 15:30"),
            }
        );
        assert_eq!(
            *day_avails.get(2).unwrap(),
            Availability {
                start: create_local_datetime("10-06-2022 16:00"),
                end: create_local_datetime("10-06-2022 17:00"),
            }
        );
    }
}
