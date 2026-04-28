use std::{collections::BTreeSet, sync::Arc};

use chrono::{prelude::*, Duration};
use colored::Colorize;
use copypasta::{ClipboardContext, ClipboardProvider};
use dialoguer::{theme::ColorfulTheme, Confirm, Input, MultiSelect, Select};
use indicatif::ProgressBar;
use itertools::Itertools;
use sesame::critical::{CriticalRegion, Signature};
use sesame::fold::fold;
use sesame::pcon::PCon;
use sesame::policy::AnyPolicyClone;
use tokio::{sync::Semaphore, task::JoinHandle};

use crate::cli::ProgressIndicator;
use crate::datetime::{
    availability::{
        format_availability, merge_overlapping_avails, split_availability, Availability,
    },
    finder::AvailabilityFinder,
};
use crate::events::{google, microsoft, Calendar, GetResources};
use crate::ifc::{
    authorized_context, compute_availability, owners_from_availability, protect_calendar,
    ProtectedAvailability, ProtectedEvents, RevealRoute,
};
use crate::store::{AccountModel, CalendarModel, Platform, Store, PLATFORMS};
use crate::util::AvailConfig;

pub fn publish_availability_output(avails: &ProtectedAvailability) -> anyhow::Result<()> {
    let authorized_owners = owners_from_availability(avails)?;
    let mut ctx = ClipboardContext::new().map_err(|err| {
        anyhow::anyhow!("failed to access clipboard: {}", err)
    })?;

    avails
        .critical(
            authorized_context(RevealRoute::AvailabilityOutput, &authorized_owners),
            CriticalRegion::new(
                |slots: &Vec<Availability<Local>>, _| -> anyhow::Result<()> {
                    let formatted = format_availability(slots);
                    print!("{}", formatted);
                    if ctx.set_contents(formatted).is_ok() {
                        println!("\nCopied to clipboard.");
                    }
                    Ok(())
                },
                Signature {
                    username: "borvath",
                    signature: "LS0tLS1CRUdJTiBTU0ggU0lHTkFUVVJFLS0tLS0KVTFOSVUwbEhBQUFBQVFBQUFETUFBQUFMYzNOb0xXVmtNalUxTVRrQUFBQWd1S1hiSjdkTFFLaHEvMWFmbkwwQUhHVlNzSgp6UnFndUVVcHl2b2Y3TkdrTUFBQUFFWm1sc1pRQUFBQUFBQUFBR2MyaGhOVEV5QUFBQVV3QUFBQXR6YzJndFpXUXlOVFV4Ck9RQUFBRUFQTjh6Uk0yTkFiRGcvNnJVYzFHQXA2R1JIcDkwc0M5bjRmUysvbG91bVg4dUlXVzY0ZWRFa3kzVVlwcEVSc20KZkF4T3YwQ1BVMnRpcFZQZE9ubWV3SgotLS0tLUVORCBTU0ggU0lHTkFUVVJFLS0tLS0K",
                },
            ),
            (),
        )
        .map_err(|_| {
            anyhow::anyhow!(
                "Sesame blocked externalization for {}",
                RevealRoute::AvailabilityOutput
            )
        })??;
    Ok(())
}

pub async fn add_account(
    db: Store,
    email: &str,
    cfg: &AvailConfig,
    shutdown_receiver: tokio::sync::oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let selection = Select::with_theme(&ColorfulTheme::default())
        .with_prompt("Which platform would you like to add an account for?")
        .items(&PLATFORMS[..])
        .default(0)
        .interact()?;

    let selected_platform = PLATFORMS[selection];

    let accounts = db.execute(Box::new(AccountModel::get))??;
    if accounts
        .iter()
        .any(|a| a.name == email && a.platform.unwrap() == selected_platform)
    {
        return Err(anyhow::anyhow!("Account already exists with that email"));
    }

    match selected_platform {
        Platform::Microsoft => {
            let (_, refresh_token) = microsoft::get_authorization_code(
                &cfg.microsoft.to_owned().unwrap_or_default(),
                shutdown_receiver,
            )
            .await?;
            crate::store::store_token(email, &refresh_token)?;
        }
        Platform::Google => {
            let (_, refresh_token) = google::get_authorization_code(
                &cfg.google.to_owned().unwrap_or_default(),
                shutdown_receiver,
            )
            .await?;
            crate::store::store_token(email, &refresh_token)?;
        }
        _ => return Err(anyhow::anyhow!("Unsupported platform")),
    }

    let account = AccountModel {
        name: email.to_owned(),
        platform: Some(selected_platform),
        id: None,
    };
    db.execute(Box::new(move |conn| account.insert(conn)))??;
    println!("\nSuccessfully added account.");
    println!(
        "Run the \"{}\" command to update the calendars cache with this account's calendars.",
        "calendars".bold()
    );

    Ok(())
}

pub fn remove_account(db: Store, email: &str) -> anyhow::Result<()> {
    if Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Do you want to delete the account \"{}\"?", email))
        .interact()?
    {
        crate::store::delete_token(email)?;
        let account = AccountModel {
            name: email.to_owned(),
            id: None,
            platform: None,
        };
        db.execute(Box::new(move |conn| account.delete(conn)))??;
        println!("Successfully removed account.");
    }

    Ok(())
}

pub fn list_accounts(db: Store) -> anyhow::Result<()> {
    let accounts = db.execute(Box::new(AccountModel::get))??;

    if accounts.is_empty() {
        println!("Configured accounts: None");
    } else {
        println!("Configured accounts:");
        for account in accounts {
            println!(
                "- {} on {}",
                account.name.bold().blue(),
                account.platform.unwrap()
            );
        }
    }

    Ok(())
}

pub async fn refresh_calendars(db: Store, cfg: &AvailConfig) -> anyhow::Result<()> {
    let accounts = db.execute(Box::new(AccountModel::get))??;

    if accounts.is_empty() {
        return Err(anyhow::anyhow!(format!(
            "You must link accounts using the \"{}\" command before fetching calendars.",
            "accounts add".italic().bold()
        )));
    }

    for account in &accounts {
        let refresh_token = crate::store::get_token(&account.name)?;

        let account_id = account.id.unwrap().to_owned();
        let mut calendars = match account.platform.unwrap() {
            Platform::Microsoft => {
                let access_token = microsoft::refresh_access_token(
                    &cfg.microsoft.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;
                microsoft::MicrosoftGraph::get_calendars(&access_token).await?
            }
            Platform::Google => {
                let access_token = google::refresh_access_token(
                    &cfg.google.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;
                google::GoogleAPI::get_calendars(&access_token).await?
            }
            _ => return Err(anyhow::anyhow!("Unsupported platform")),
        };

        let authorized_owners = BTreeSet::from([account.name.clone()]);
        let protected_calendars: PCon<Vec<Calendar>, _> = calendars
            .iter()
            .cloned()
            .map(|calendar| protect_calendar(calendar, &account.name))
            .collect::<Vec<_>>()
            .into();

        let mut prev_unselected_calendars = db
            .execute(Box::new(move |conn| {
                CalendarModel::get_all_selected(conn, &account_id.to_owned(), false)
            }))??
            .into_iter()
            .map(|c| c.id);

        let mut defaults = vec![];
        for cal in calendars.iter() {
            defaults.push(!prev_unselected_calendars.contains(&cal.id));
        }

        let selected_calendars_idx = protected_calendars
            .critical(
                authorized_context(RevealRoute::RefreshCalendarsPrompt, &authorized_owners),
                CriticalRegion::new(
                    |calendars: &Vec<Calendar>, _| -> anyhow::Result<Vec<usize>> {
                        Ok(MultiSelect::with_theme(&ColorfulTheme::default())
                            .items(calendars)
                            .defaults(&defaults)
                            .with_prompt(format!(
                                "Select the calendars you want to use for {}",
                                account.name
                            ))
                            .interact()?)
                    },
                    Signature {
                        username: "borvath",
                        signature: "LS0tLS1CRUdJTiBTU0ggU0lHTkFUVVJFLS0tLS0KVTFOSVUwbEhBQUFBQVFBQUFETUFBQUFMYzNOb0xXVmtNalUxTVRrQUFBQWd1S1hiSjdkTFFLaHEvMWFmbkwwQUhHVlNzSgp6UnFndUVVcHl2b2Y3TkdrTUFBQUFFWm1sc1pRQUFBQUFBQUFBR2MyaGhOVEV5QUFBQVV3QUFBQXR6YzJndFpXUXlOVFV4Ck9RQUFBRUEwaXBOZ3k4Y3l4RHNvdnl5U1pyL3ZBMDVkMDdPQ3F5MVY3U3piM0FVYkg3UitMRGYzcTJHY2VBSWx0REhvL0IKSHFDNWRiYU5nL1BiRTQ1dkZOSWVRSwotLS0tLUVORCBTU0ggU0lHTkFUVVJFLS0tLS0K",
                    },
                ),
                (),
            )
            .map_err(|_| {
                anyhow::anyhow!(
                    "Sesame blocked externalization for {}",
                    RevealRoute::RefreshCalendarsPrompt
                )
            })??;

        for (i, cal) in calendars.iter_mut().enumerate() {
            cal.selected = selected_calendars_idx.contains(&i);
        }

        db.execute(Box::new(move |conn| {
            CalendarModel::delete_for_account(conn, &account_id)
        }))??;

        let insert_calendars: Vec<CalendarModel> = calendars
            .into_iter()
            .map(|c| CalendarModel {
                account_id: account.id,
                id: c.id,
                name: c.name,
                selected: c.selected,
            })
            .collect();

        db.execute(Box::new(|conn| {
            CalendarModel::insert_many(conn, insert_calendars)
        }))??;
    }

    let all_owners: BTreeSet<String> = accounts
        .iter()
        .map(|account| account.name.clone())
        .collect();

    let mut all_calendars: Vec<Calendar> = db
        .execute(Box::new(CalendarModel::get_all))??
        .into_iter()
        .map(|c| Calendar {
            account_id: c.account_id.unwrap(),
            id: c.id,
            name: c.name,
            selected: false,
        })
        .collect();

    let protected_all_calendars: PCon<Vec<Calendar>, _> = all_calendars
        .iter()
        .cloned()
        .map(|calendar| {
            let owner = accounts
                .iter()
                .find(|account| account.id == Some(calendar.account_id))
                .map(|account| account.name.as_str())
                .expect("calendar owner must exist");
            protect_calendar(calendar, owner)
        })
        .collect::<Vec<_>>()
        .into();

    let previous_selected = db.execute(Box::new(move |conn| {
        CalendarModel::get_hold_event_calendar(conn)
    }))??;

    let previous_selected_idx: usize = if let Some((_, cal)) = previous_selected {
        let e = all_calendars.iter().enumerate().find(|e| e.1.id == cal.id);
        e.unwrap().0
    } else {
        0
    };

    let selected_calendar_idx = protected_all_calendars
        .critical(
            authorized_context(RevealRoute::RefreshCalendarsHoldEventTarget, &all_owners),
            CriticalRegion::new(
                |calendars: &Vec<Calendar>, _| -> anyhow::Result<usize> {
                    Ok(Select::with_theme(&ColorfulTheme::default())
                        .items(calendars)
                        .default(previous_selected_idx)
                        .with_prompt("Which calendar would you like to use to create hold events?")
                        .interact()?)
                },
                Signature {
                    username: "borvath",
                    signature: "LS0tLS1CRUdJTiBTU0ggU0lHTkFUVVJFLS0tLS0KVTFOSVUwbEhBQUFBQVFBQUFETUFBQUFMYzNOb0xXVmtNalUxTVRrQUFBQWd1S1hiSjdkTFFLaHEvMWFmbkwwQUhHVlNzSgp6UnFndUVVcHl2b2Y3TkdrTUFBQUFFWm1sc1pRQUFBQUFBQUFBR2MyaGhOVEV5QUFBQVV3QUFBQXR6YzJndFpXUXlOVFV4Ck9RQUFBRUNjcUZBd3gxNkJ6aXJKSUt5ZWhwT0JhaFREYXcrS25CbXc0d25Ua3FXZkdKWEdnY3lJMTljdDZZVlpnb0I5ZlEKcHRIVXowZElLRzhzWE82ejVPRlk0SgotLS0tLUVORCBTU0ggU0lHTkFUVVJFLS0tLS0K",
                },
            ),
            (),
        )
        .map_err(|_| {
            anyhow::anyhow!(
                "Sesame blocked externalization for {}",
                RevealRoute::RefreshCalendarsHoldEventTarget
            )
        })??;

    let selected_calendar = all_calendars.get_mut(selected_calendar_idx).unwrap();
    selected_calendar.selected = true;

    let update_calendar = CalendarModel {
        account_id: Some(selected_calendar.account_id),
        id: selected_calendar.id.to_owned(),
        name: selected_calendar.name.to_owned(),
        selected: true,
    };

    db.execute(Box::new(move |conn| {
        CalendarModel::update_hold_event_calendar(conn, update_calendar)
    }))??;

    Ok(())
}

pub(crate) async fn find_availability(
    db: &Store,
    cfg: &AvailConfig,
    finder: AvailabilityFinder,
    m: &ProgressIndicator,
) -> anyhow::Result<Option<ProtectedAvailability>> {
    let accounts = db.execute(Box::new(AccountModel::get))??;

    if accounts.is_empty() {
        return Err(anyhow::anyhow!(format!(
            "You must link accounts using the \"{}\" command and configure calendars using \"{}\" command before you are able to find availabilities.",
            "accounts add".bold().italic(),
            "calendars".bold().italic()
        )));
    }

    println!(
        "Finding availability between {} and {}\n",
        format!("{}", finder.start.format("%b %-d %Y"))
            .bold()
            .blue(),
        format!("{}", finder.end.format("%b %-d %Y")).bold().blue()
    );

    let pb = m.add(ProgressBar::new(1));
    pb.set_message("Retrieving events...");
    pb.enable_steady_tick(Duration::milliseconds(250).to_std()?);

    // Microsoft Graph has 4 concurrent requests limit
    let semaphore = Arc::new(Semaphore::new(4));
    let mut tasks: Vec<JoinHandle<anyhow::Result<ProtectedEvents>>> = vec![];
    let mut authorized_owners = BTreeSet::new();

    for account in accounts {
        let account_id = account.id.unwrap().to_owned();
        let owner = account.name.clone();
        let selected_calendars = db
            .execute(Box::new(move |conn| {
                CalendarModel::get_all_selected(conn, &account_id, true)
            }))??
            .into_iter()
            .collect_vec();

        if !selected_calendars.is_empty() {
            authorized_owners.insert(owner.clone());
        }

        match account.platform.unwrap() {
            Platform::Microsoft => {
                let refresh_token = crate::store::get_token(&account.name)?;
                let access_token = microsoft::refresh_access_token(
                    &cfg.microsoft.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;

                for calendar in selected_calendars {
                    let token = access_token.clone();
                    let cal_id = calendar.id;
                    let owner = owner.clone();
                    let permit = semaphore
                        .clone()
                        .acquire_owned()
                        .await
                        .expect("unable to acquire permit"); // Acquire a permit
                    tasks.push(tokio::task::spawn(async move {
                        let res = microsoft::MicrosoftGraph::get_calendar_events(
                            &token,
                            &cal_id,
                            &owner,
                            finder.start,
                            finder.end,
                        )
                        .await?;
                        drop(permit);
                        Ok(res)
                    }));
                }
            }
            Platform::Google => {
                let refresh_token = crate::store::get_token(&account.name)?;
                let access_token = google::refresh_access_token(
                    &cfg.google.to_owned().unwrap_or_default(),
                    &refresh_token,
                )
                .await?;

                for calendar in selected_calendars {
                    let token = access_token.clone();
                    let cal_id = calendar.id;
                    let owner = owner.clone();
                    tasks.push(tokio::task::spawn(async move {
                        let res = google::GoogleAPI::get_calendar_events(
                            &token,
                            &cal_id,
                            &owner,
                            finder.start,
                            finder.end,
                        )
                        .await?;
                        Ok(res)
                    }));
                }
            }
            _ => return Err(anyhow::anyhow!("Unsupported platform")),
        }
    }

    if authorized_owners.is_empty() {
        return Err(anyhow::anyhow!(
            "No calendars are selected. Run the \"{}\" command to choose calendars first.",
            "calendars".bold().italic()
        ));
    }

    let protected_events = futures::future::join_all(tasks)
        .await
        .into_iter()
        .filter_map(|r| r.ok())
        .collect::<anyhow::Result<Vec<_>>>()?;

    pb.finish_with_message("Retrieved events.");

    let pb = m.add(ProgressBar::new(1));
    pb.set_message("Computing availabilities...");
    pb.enable_steady_tick(Duration::milliseconds(250).to_std()?);

    let protected_slots = compute_availability(&finder, protected_events)?;
    let authorized_owners = owners_from_availability(&protected_slots)?;
    let availability_policy = protected_slots.policy().clone();

    pb.finish_with_message("Computed availabilities.");

    protected_slots
        .into_critical(
            authorized_context(RevealRoute::AvailabilitySelection, &authorized_owners),
            CriticalRegion::new(
                move |slots: Vec<Availability<Local>>, _| -> anyhow::Result<Option<ProtectedAvailability>> {
                    if slots.is_empty() {
                        return Ok(None);
                    }

                    // TODO: add multi-level multiselect
                    // Right arrow goes into a time window (can select granular windows)
                    // Left arrow goes back to parent
                    // Needs to work with paging
                    let selection = MultiSelect::with_theme(&ColorfulTheme::default())
                        .with_prompt("Select time window(s)")
                        .items(&slots)
                        .interact()?;

                    let selected_slots = selection.into_iter().map(|i| slots.get(i).unwrap());
                    let days = selected_slots.group_by(|e| e.start.date());
                    let mut iter = days.into_iter().peekable();
                    let mut selected: Vec<Availability<Local>> = vec![];

                    while iter.peek().is_some() {
                        let (day, avails) = iter.next().unwrap();

                        let day_slots: Vec<&Availability<Local>> = avails.into_iter().collect();
                        let windows = split_availability(&day_slots, finder.duration);

                        let selection = MultiSelect::with_theme(&ColorfulTheme::default())
                            .with_prompt(format!(
                                "Select time window(s) for {}",
                                day.format("%b %d %Y")
                            ))
                            .items(&windows)
                            .interact()?;

                        let mut selected_windows: Vec<Availability<Local>> = selection
                            .into_iter()
                            .map(|i| *windows.get(i).unwrap())
                            .collect();
                        selected.append(&mut selected_windows);
                    }

                    if selected.is_empty() {
                        return Err(anyhow::anyhow!("No availabilities selected."));
                    }

                    Ok(Some(PCon::new(
                        merge_overlapping_avails(selected),
                        availability_policy,
                    )))
                },
                Signature {
                    username: "borvath",
                    signature: "LS0tLS1CRUdJTiBTU0ggU0lHTkFUVVJFLS0tLS0KVTFOSVUwbEhBQUFBQVFBQUFETUFBQUFMYzNOb0xXVmtNalUxTVRrQUFBQWd1S1hiSjdkTFFLaHEvMWFmbkwwQUhHVlNzSgp6UnFndUVVcHl2b2Y3TkdrTUFBQUFFWm1sc1pRQUFBQUFBQUFBR2MyaGhOVEV5QUFBQVV3QUFBQXR6YzJndFpXUXlOVFV4Ck9RQUFBRUE5NXltUFNDKy9Zd2pSTndsRjN6UDQycTJ3VVVTdWlqQ055bDRaSkFBek1MSUwxd3hqc3J6NE1adzBNd1dncTIKOFZ6RUVJUGEwUjZyQS9tUE8xMGxzTwotLS0tLUVORCBTU0ggU0lHTkFUVVJFLS0tLS0K",
                },
            ),
            (),
        )
        .map_err(|_| {
            anyhow::anyhow!(
                "Sesame blocked externalization for {}",
                RevealRoute::AvailabilitySelection
            )
        })?
}

pub(crate) async fn create_hold_events(
    db: Store,
    cfg: &AvailConfig,
    merged: &ProtectedAvailability,
    m: &ProgressIndicator,
) -> anyhow::Result<()> {
    let accounts = db.execute(Box::new(AccountModel::get))??;
    let authorized_owners = owners_from_availability(merged)?;

    let event_title: String = Input::with_theme(&ColorfulTheme::default())
        .with_prompt("What's the name of your event?")
        .interact_text()?;

    let calendar = db.execute(Box::new(move |conn| {
        CalendarModel::get_hold_event_calendar(conn)
    }))??;

    if calendar.is_none() {
        return Err(anyhow::anyhow!(
            "No calendar is configured to be used for hold events."
        ));
    }

    let pb = m.add(ProgressBar::new(1));
    pb.set_message("Creating hold events...");
    pb.enable_steady_tick(Duration::milliseconds(250).to_std()?);

    let (platform, cal) = calendar.unwrap();

    // Microsoft Graph has 4 concurrent requests limit
    let semaphore = Arc::new(Semaphore::new(4));

    let account_name = accounts
        .iter()
        .find(|a| a.id == cal.account_id)
        .unwrap()
        .name
        .to_owned();
    let protected_hold_calendar = protect_calendar(
        Calendar {
            account_id: cal.account_id.unwrap(),
            id: cal.id.clone(),
            name: cal.name.clone(),
            selected: true,
        },
        &account_name,
    );
    let platform = Platform::from(&platform);
    let refresh_token = crate::store::get_token(&account_name)?;
    let access_token = match platform {
        Platform::Microsoft => {
            microsoft::refresh_access_token(&cfg.microsoft.to_owned().unwrap_or_default(), &refresh_token)
                .await?
        }
        Platform::Google => {
            google::refresh_access_token(&cfg.google.to_owned().unwrap_or_default(), &refresh_token)
                .await?
        }
        Platform::Unsupported => return Err(anyhow::anyhow!("Unsupported platform")),
    };

    let protected_hold_data: PCon<
        (Calendar, Vec<Availability<Local>>),
        AnyPolicyClone,
    > = fold((
        protected_hold_calendar.into_any_policy(),
        merged.clone(),
    ))
    .map_err(|_| anyhow::anyhow!("failed to combine protected hold-event data"))?;

    protected_hold_data
        .critical(
            authorized_context(RevealRoute::HoldEventsCreate, &authorized_owners),
            CriticalRegion::new(
                |(calendar, slots): &(Calendar, Vec<Availability<Local>>), _| -> anyhow::Result<()> {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            let mut tasks: Vec<JoinHandle<anyhow::Result<()>>> = vec![];

                            for avail in slots.iter() {
                                let permit = match platform {
                                    Platform::Microsoft => Some(
                                        semaphore
                                            .clone()
                                            .acquire_owned()
                                            .await
                                            .expect("unable to acquire permit"),
                                    ),
                                    Platform::Google => None,
                                    Platform::Unsupported => {
                                        return Err(anyhow::anyhow!("Unsupported platform"))
                                    }
                                };
                                let calendar_id = calendar.id.clone();
                                let title = format!("HOLD - {}", event_title);
                                let start = avail.start;
                                let end = avail.end;
                                let access_token = access_token.clone();

                                tasks.push(tokio::task::spawn(async move {
                                    let res = match platform {
                                        Platform::Microsoft => {
                                            microsoft::MicrosoftGraph::create_event(
                                                &access_token,
                                                &calendar_id,
                                                &title,
                                                start,
                                                end,
                                            )
                                            .await
                                        }
                                        Platform::Google => {
                                            google::GoogleAPI::create_event(
                                                &access_token,
                                                &calendar_id,
                                                &title,
                                                start,
                                                end,
                                            )
                                            .await
                                        }
                                        Platform::Unsupported => {
                                            Err(anyhow::anyhow!("Unsupported platform"))
                                        }
                                    };
                                    drop(permit);
                                    res?;
                                    Ok(())
                                }));
                            }

                            let res = futures::future::join_all(tasks).await;
                            if res.iter().any(|r| r.is_err()) {
                                return Err(anyhow::anyhow!("Failed to create hold events."));
                            }
                            Ok(())
                        })
                    })
                },
                Signature {
                    username: "borvath",
                    signature: "LS0tLS1CRUdJTiBTU0ggU0lHTkFUVVJFLS0tLS0KVTFOSVUwbEhBQUFBQVFBQUFETUFBQUFMYzNOb0xXVmtNalUxTVRrQUFBQWd1S1hiSjdkTFFLaHEvMWFmbkwwQUhHVlNzSgp6UnFndUVVcHl2b2Y3TkdrTUFBQUFFWm1sc1pRQUFBQUFBQUFBR2MyaGhOVEV5QUFBQVV3QUFBQXR6YzJndFpXUXlOVFV4Ck9RQUFBRUI0Y2p2WHRkbFRtd2x1K3EwV0hOWVZVZU5VcTIvLzdKeDlZRldpMDB4OFRIQy9wNHpvYjUzWkZMOHRJV0FKMnUKNTdpRTlHbXZWeVpNUGpZdktHRnlNUAotLS0tLUVORCBTU0ggU0lHTkFUVVJFLS0tLS0K",
                },
            ),
            (),
        )
        .map_err(|_| {
            anyhow::anyhow!(
                "Sesame blocked externalization for {}",
                RevealRoute::HoldEventsCreate
            )
        })??;

    pb.finish_with_message("Created hold events.");

    Ok(())
}
