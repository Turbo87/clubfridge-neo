use crate::database;
use crate::state::{GlobalState, Message};
use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::{Subscription, Task};
use rust_decimal::Decimal;
use sqlx::types::Text;
use sqlx::SqlitePool;
use std::mem;
use std::ops::Sub;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};
use ulid::Ulid;

/// The interval at which the app should load articles and users from
/// the Vereinsflieger API.
const SYNC_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// The interval at which the app should upload new sales to
/// the Vereinsflieger API.
const SALES_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// The time after which the sale is automatically processed.
const INTERACTION_TIMEOUT: jiff::SignedDuration = jiff::SignedDuration::from_secs(60);

pub struct RunningClubFridge {
    pub pool: SqlitePool,
    pub vereinsflieger: Option<vereinsflieger::Client>,
    /// Mutex to ensure that only one upload task runs at a time.
    pub upload_mutex: Arc<tokio::sync::Mutex<()>>,

    pub user: Option<database::Member>,
    pub input: String,
    pub sales: Vec<Sale>,
    pub interaction_timeout: Option<jiff::SignedDuration>,
}

impl RunningClubFridge {
    pub fn new(
        pool: SqlitePool,
        vereinsflieger: Option<vereinsflieger::Client>,
    ) -> (Self, Task<Message>) {
        let mut tasks = vec![];
        if vereinsflieger.is_some() {
            tasks.push(Task::done(Message::LoadFromVF));
            tasks.push(Task::done(Message::UploadSalesToVF));
        } else {
            info!("Running in offline mode, skipping Vereinsflieger sync");
        }

        let cf = Self {
            pool,
            vereinsflieger,
            upload_mutex: Default::default(),
            user: None,
            input: String::new(),
            sales: Vec::new(),
            interaction_timeout: None,
        };

        (cf, Task::batch(tasks))
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![iced::keyboard::listen().filter_map(|event| {
            if let iced::keyboard::Event::KeyPressed { key, modifiers, .. } = event {
                Some(Message::KeyPress(key, modifiers))
            } else {
                None
            }
        })];

        if self.vereinsflieger.is_some() {
            subscriptions.push(iced::time::every(SYNC_INTERVAL).map(|_| Message::LoadFromVF));
            subscriptions.push(iced::time::every(SALES_INTERVAL).map(|_| Message::UploadSalesToVF));
        }

        if self.interaction_timeout.is_some() {
            subscriptions
                .push(iced::time::every(Duration::from_secs(1)).map(|_| Message::DecrementTimeout));
        }

        Subscription::batch(subscriptions)
    }
}

#[derive(Debug, Clone)]
pub struct Sale {
    pub amount: u16,
    pub article: database::Article,
}

impl Sale {
    pub fn total(&self) -> Decimal {
        Decimal::from(self.amount) * self.article.current_price().unwrap_or_default()
    }
}

impl RunningClubFridge {
    pub fn update(&mut self, message: Message, global_state: &mut GlobalState) -> Task<Message> {
        match message {
            Message::LoadFromVF => {
                let Some(vereinsflieger) = &self.vereinsflieger else {
                    return Task::none();
                };

                let vf_clone = vereinsflieger.clone();
                let pool_clone = self.pool.clone();
                let load_articles_task = Task::future(async move {
                    info!("Loading articles from Vereinsflieger API…");
                    let articles = vf_clone.list_articles().await?;
                    info!(
                        "Received {} articles from Vereinsflieger API",
                        articles.len()
                    );

                    let articles = articles
                        .into_iter()
                        .filter_map(|article| {
                            database::Article::try_from(article)
                                .inspect_err(|err| warn!("Found invalid article: {err}"))
                                .ok()
                        })
                        .collect::<Vec<_>>();

                    info!("Saving {} articles to database…", articles.len());
                    database::Article::save_all(pool_clone, articles).await?;

                    Ok::<_, anyhow::Error>(())
                })
                .then(|result| {
                    match result {
                        Ok(_) => info!("Articles successfully saved to database"),
                        Err(err) => error!("Failed to load articles: {err}"),
                    }

                    Task::none()
                });

                let vf_clone = vereinsflieger.clone();
                let pool_clone = self.pool.clone();
                let load_members_task = Task::future(async move {
                    info!("Loading users from Vereinsflieger API…");
                    let users = vf_clone.list_users().await?;
                    info!("Received {} users from Vereinsflieger API", users.len());

                    let users = users
                        .into_iter()
                        .flat_map(|user| {
                            user.keymanagement
                                .into_iter()
                                .filter_map(database::Member::parse_keycode)
                                .map(move |keycode| database::Member {
                                    keycode,
                                    id: user.member_id.clone(),
                                    firstname: user.first_name.clone(),
                                    lastname: user.last_name.clone(),
                                    nickname: user.nickname.clone(),
                                })
                        })
                        .collect::<Vec<_>>();

                    info!("Saving {} users with keycodes to database…", users.len());
                    database::Member::save_all(pool_clone, users).await?;

                    Ok::<_, anyhow::Error>(())
                })
                .then(|result| {
                    match result {
                        Ok(_) => info!("Users successfully saved to database"),
                        Err(err) => error!("Failed to load users: {err}"),
                    }

                    Task::none()
                });

                return Task::batch([load_articles_task, load_members_task]);
            }
            Message::UploadSalesToVF => {
                let Some(vereinsflieger) = &self.vereinsflieger else {
                    return Task::none();
                };

                let vereinsflieger = vereinsflieger.clone();
                let pool = self.pool.clone();
                let upload_mutex = self.upload_mutex.clone();

                return Task::future(async move {
                    let _guard = upload_mutex.lock().await;

                    info!("Loading sales from database…");
                    let sales = database::Sale::load_all(pool.clone()).await?;
                    if sales.is_empty() {
                        info!("No sales to upload");
                        return Ok(());
                    }

                    info!("Uploading {} sales to Vereinsflieger API…", sales.len());
                    for (i, sale) in sales.into_iter().enumerate() {
                        let sale_id = *sale.id;
                        debug!(%sale_id, "Uploading sale #{}…", i + 1);

                        async fn save_sale(
                            vereinsflieger: &vereinsflieger::Client,
                            sale: database::Sale,
                        ) -> Result<(), anyhow::Error> {
                            let sale = vereinsflieger::NewSale {
                                booking_date: &sale.date.to_string(),
                                article_id: &sale.article_id,
                                amount: sale.amount as f64,
                                member_id: Some(sale.member_id.parse()?),
                                callsign: None,
                                sales_tax: None,
                                total_price: None,
                                counter: None,
                                comment: None,
                                cost_type: None,
                                caid2: None,
                                spid: None,
                            };

                            Ok(vereinsflieger.add_sale(&sale).await?)
                        }

                        if let Err(error) = save_sale(&vereinsflieger, sale).await {
                            warn!(%sale_id, "Failed to upload sale: {error}");
                        } else {
                            debug!(%sale_id, "Deleting sale from database…");
                            match database::Sale::delete_by_id(&pool, sale_id).await {
                                Ok(()) => debug!(%sale_id, "Sale successfully deleted"),
                                Err(err) => warn!(%sale_id, "Failed to delete sale: {err}"),
                            }
                        }
                    }

                    Ok::<_, anyhow::Error>(())
                })
                .then(|result| {
                    match result {
                        Ok(_) => info!("Sales successfully uploaded"),
                        Err(err) => error!("Failed to upload sales: {err}"),
                    }

                    Task::none()
                });
            }
            Message::KeyPress(Key::Character(c), modifiers) => {
                let mut c = c.chars().next().unwrap();
                if modifiers.shift() {
                    c = c.to_ascii_uppercase();
                }

                debug!("Key pressed: {c:?}");
                self.input.push(c);
                global_state.hide_popup();
            }
            Message::KeyPress(Key::Named(Named::Enter), _) => {
                debug!("Key pressed: Enter");
                let input = mem::take(&mut self.input);
                let pool = self.pool.clone();

                global_state.hide_popup();

                return if self.user.is_some() {
                    Task::future(async move {
                        let result = database::Article::find_by_barcode(pool, &input).await;
                        let result = result.map_err(Arc::new);
                        Message::FindArticleResult { input, result }
                    })
                } else {
                    Task::future(async move {
                        let result = database::Member::find_by_keycode(pool, &input).await;
                        let result = result.map_err(Arc::new);
                        Message::FindMemberResult { input, result }
                    })
                };
            }
            #[cfg(debug_assertions)]
            Message::KeyPress(Key::Named(Named::Control), _) => {
                use rust_decimal_macros::dec;

                let task = if self.user.is_some() {
                    let ulid = Ulid::gen();

                    let timestamp = ulid.timestamp_ms();

                    let designations = [
                        "Testartikel 1",
                        "Testartikel 2 asd nflkjdbnslf kjalsdk fj lkdjsnlfkjnaldsknf lknlksdanfl kndslkf nlkaflkn a",
                        "Test",
                    ];
                    let n = timestamp % designations.len() as u64;

                    let ulid = ulid.to_string();
                    Task::done(Message::FindArticleResult {
                        input: ulid.clone(),
                        result: Ok(Some(database::Article {
                            id: designations[n as usize].to_string(),
                            designation: designations[n as usize].to_string(),
                            prices: vec![{
                                database::Price {
                                    valid_from: jiff::civil::Date::constant(2000, 1, 1),
                                    valid_to: jiff::civil::Date::constant(2999, 12, 31),
                                    unit_price: Decimal::from(timestamp % 1000) / dec!(100),
                                }
                            }],
                        })),
                    })
                } else {
                    Task::done(Message::FindMemberResult {
                        input: "1234567890".to_string(),
                        result: Ok(Some(database::Member {
                            keycode: "1234567890".to_string(),
                            id: "11011".to_string(),
                            firstname: "Tobias".to_string(),
                            lastname: "Bieniek".to_string(),
                            nickname: "Turbo".to_string(),
                        })),
                    })
                };

                global_state.hide_popup();

                return task;
            }
            Message::FindArticleResult { input, result } => match result {
                Ok(Some(article)) => {
                    info!("Adding article to sale: {article:?}");
                    if self.user.is_some() && article.current_price().is_some() {
                        let sales = &mut self.sales;

                        let existing_sale =
                            sales.iter_mut().find(|item| item.article.id == article.id);
                        match existing_sale {
                            Some(item) => item.amount += 1,
                            None => sales.push(Sale { amount: 1, article }),
                        }

                        self.interaction_timeout = Some(INTERACTION_TIMEOUT);
                    }
                }
                Ok(None) => {
                    warn!("No article found for barcode: {input}");
                    return global_state.show_popup(format!("Artikel nicht gefunden ({input})"));
                }
                Err(err) => {
                    error!("Failed to find article: {err}");
                }
            },
            Message::FindMemberResult { input, result } => match result {
                Ok(Some(member)) => {
                    info!("Setting user: {member:?}");
                    self.user = Some(member);
                    self.interaction_timeout = Some(INTERACTION_TIMEOUT);
                }
                Ok(None) => {
                    warn!("No user found for keycode: {input}");
                    return global_state.show_popup(format!("Benutzer nicht gefunden ({input})"));
                }
                Err(err) => {
                    error!("Failed to find user: {err}");
                }
            },
            Message::DecrementTimeout => {
                if let Some(timeout) = &mut self.interaction_timeout {
                    *timeout = timeout.sub(jiff::SignedDuration::from_secs(1));
                    if timeout.is_zero() {
                        info!("Interaction timeout reached");
                        self.interaction_timeout = None;
                        return Task::done(if self.sales.is_empty() {
                            Message::Cancel
                        } else {
                            Message::Pay
                        });
                    }
                }
            }
            Message::Pay => {
                info!("Processing sale");
                let pool = self.pool.clone();
                let date = jiff::Zoned::now().date();

                let sales = mem::take(&mut self.sales)
                    .into_iter()
                    .map(|item| database::Sale {
                        id: Text(Ulid::gen()),
                        date: Text(date),
                        member_id: self
                            .user
                            .as_ref()
                            .map(|user| &user.id)
                            .cloned()
                            .unwrap_or_default(),
                        article_id: item.article.id,
                        amount: item.amount as u32,
                    })
                    .collect();

                self.interaction_timeout = None;

                return Task::future(database::Sale::insert_all(pool, sales)).then(|result| {
                    match result {
                        Ok(()) => Task::batch([
                            Task::done(Message::SalesSaved),
                            Task::done(Message::UploadSalesToVF),
                        ]),
                        Err(err) => {
                            error!("Failed to save sales: {err}");
                            Task::done(Message::SavingSalesFailed)
                        }
                    }
                });
            }
            Message::SalesSaved => {
                info!("Sales saved");
                self.user = None;
                self.sales.clear();
                return global_state.show_popup("Danke für deinen Kauf");
            }
            Message::SavingSalesFailed => {
                error!("Failed to save sales");
            }
            Message::Cancel => {
                info!("Cancelling sale");
                self.user = None;
                self.sales.clear();
                self.interaction_timeout = None;
            }
            _ => {}
        }

        Task::none()
    }
}
