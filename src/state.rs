use crate::config::Config;
use iced::keyboard::key::Named;
use iced::keyboard::Key;
use iced::Task;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::time::Duration;

pub struct State {
    #[allow(dead_code)]
    pub config: Config,

    pub articles: HashMap<String, Article>,
    pub users: HashMap<String, String>,

    pub user: Option<String>,
    pub input: String,
    pub items: Vec<Item>,
    pub show_sale_confirmation: bool,
}

impl State {
    pub fn from_config(config: Config) -> State {
        let articles = vec![
            Article {
                ean: "3800235265659".to_string(),
                description: "Gloriette Cola Mix".to_string(),
                prices: vec![Price {
                    valid_from: jiff::civil::date(2000, 1, 1),
                    valid_to: jiff::civil::date(2999, 12, 31),
                    unit_price: dec!(0.9),
                }],
            },
            Article {
                ean: "x001wfi0uh".to_string(),
                description: "Bratwurst".to_string(),
                prices: vec![Price {
                    valid_from: jiff::civil::date(2000, 1, 1),
                    valid_to: jiff::civil::date(2999, 12, 31),
                    unit_price: dec!(1.5),
                }],
            },
            Article {
                ean: "3800235266700".to_string(),
                description: "Erdinger Weissbier 0.5L".to_string(),
                prices: vec![Price {
                    valid_from: jiff::civil::date(2000, 1, 1),
                    valid_to: jiff::civil::date(2999, 12, 31),
                    unit_price: dec!(1.2),
                }],
            },
        ];
        let articles = HashMap::from_iter(
            articles
                .into_iter()
                .map(|article| (article.ean.clone(), article)),
        );
        let users = HashMap::from([("0005635570".to_string(), "Tobias Bieniek".to_string())]);

        Self {
            config,
            articles,
            users,
            user: None,
            input: String::new(),
            items: vec![],
            show_sale_confirmation: false,
        }
    }
}

#[derive(Debug)]
pub struct Article {
    pub ean: String,
    pub description: String,
    pub prices: Vec<Price>,
}

impl Article {
    pub fn current_price(&self) -> Option<Decimal> {
        self.price_for_date(&jiff::Zoned::now().date())
    }

    pub fn price_for_date(&self, date: &jiff::civil::Date) -> Option<Decimal> {
        self.prices
            .iter()
            .find(|price| price.valid_from <= *date && price.valid_to >= *date)
            .map(|price| price.unit_price)
    }
}

#[derive(Debug)]
pub struct Price {
    pub valid_from: jiff::civil::Date,
    pub valid_to: jiff::civil::Date,
    pub unit_price: Decimal,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub ean: String,
    pub amount: u16,
    pub description: String,
    pub price: Decimal,
}

impl Item {
    pub fn total(&self) -> Decimal {
        self.price * Decimal::from(self.amount)
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    KeyPress(Key),
    SetUser { keycode: String },
    AddToSale { barcode: String },
    Pay,
    Cancel,
    HideSaleConfirmation,
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::KeyPress(Key::Character(c)) => {
            state.input.push_str(c.as_str());
            state.show_sale_confirmation = false;
        }
        Message::KeyPress(Key::Named(Named::Enter)) => {
            let task = if state.user.is_some() {
                let barcode = state.input.clone();
                Task::done(Message::AddToSale { barcode })
            } else {
                let keycode = state.input.clone();
                Task::done(Message::SetUser { keycode })
            };

            state.input.clear();
            state.show_sale_confirmation = false;

            return task;
        }
        #[cfg(debug_assertions)]
        Message::KeyPress(Key::Named(Named::Control)) => {
            let task = if state.user.is_some() {
                let barcode = state.articles.values().next().unwrap().ean.clone();
                Task::done(Message::AddToSale { barcode })
            } else {
                let keycode = state.users.keys().next().unwrap().clone();
                Task::done(Message::SetUser { keycode })
            };

            state.show_sale_confirmation = false;

            return task;
        }
        Message::AddToSale { barcode } => {
            if state.user.is_some() {
                if let Some(article) = state.articles.get(&barcode) {
                    if let Some(price) = article.current_price() {
                        state
                            .items
                            .iter_mut()
                            .find(|item| item.ean == article.ean)
                            .map(|item| {
                                item.amount += 1;
                            })
                            .unwrap_or_else(|| {
                                state.items.push(Item {
                                    ean: article.ean.clone(),
                                    amount: 1,
                                    description: article.description.clone(),
                                    price,
                                });
                            });
                    }
                }
            }
        }
        Message::SetUser { keycode } => {
            if state.users.contains_key(&keycode) {
                state.user = Some(keycode);
            }
        }
        Message::Pay => {
            state.user = None;
            state.items.clear();
            state.show_sale_confirmation = true;
            return Task::perform(tokio::time::sleep(Duration::from_secs(3)), |_| {
                Message::HideSaleConfirmation
            });
        }
        Message::Cancel => {
            state.user = None;
            state.items.clear();
        }
        Message::HideSaleConfirmation => {
            state.show_sale_confirmation = false;
        }
        _ => {}
    }

    Task::none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(state: &mut State, input: &str) {
        for c in input.chars() {
            let char = c.to_string().into();
            let _ = update(state, Message::KeyPress(Key::Character(char)));
        }

        let _ = update(state, Message::KeyPress(Key::Named(Named::Enter)));
    }

    #[test]
    fn test_initial_state() {
        let state = State::from_config(Config::dummy());
        assert_eq!(state.user, None);
        assert_eq!(state.input, "");
        assert_eq!(state.items.len(), 0);
        assert!(!state.show_sale_confirmation);
    }

    #[tokio::test]
    async fn test_happy_path() {
        let mut state = State::from_config(Config::dummy());

        input(&mut state, "0005635570");
        assert_eq!(state.user.as_deref().unwrap_or_default(), "0005635570");
        assert_eq!(state.items.len(), 0);
        assert!(!state.show_sale_confirmation);

        input(&mut state, "3800235265659");
        assert_eq!(state.user.as_deref().unwrap_or_default(), "0005635570");
        assert_eq!(state.items.len(), 1);
        assert_eq!(state.items[0].ean, "3800235265659");
        assert_eq!(state.items[0].description, "Gloriette Cola Mix");
        assert_eq!(state.items[0].amount, 1);
        assert_eq!(state.items[0].price, dec!(0.9));
        assert!(!state.show_sale_confirmation);

        input(&mut state, "3800235266700");
        assert_eq!(state.user.as_deref().unwrap_or_default(), "0005635570");
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].ean, "3800235265659");
        assert_eq!(state.items[0].description, "Gloriette Cola Mix");
        assert_eq!(state.items[0].amount, 1);
        assert_eq!(state.items[0].price, dec!(0.9));
        assert_eq!(state.items[1].ean, "3800235266700");
        assert_eq!(state.items[1].description, "Erdinger Weissbier 0.5L");
        assert_eq!(state.items[1].amount, 1);
        assert_eq!(state.items[1].price, dec!(1.2));
        assert!(!state.show_sale_confirmation);

        input(&mut state, "3800235265659");
        assert_eq!(state.user.as_deref().unwrap_or_default(), "0005635570");
        assert_eq!(state.items.len(), 2);
        assert_eq!(state.items[0].ean, "3800235265659");
        assert_eq!(state.items[0].description, "Gloriette Cola Mix");
        assert_eq!(state.items[0].amount, 2);
        assert_eq!(state.items[0].price, dec!(0.9));
        assert_eq!(state.items[1].ean, "3800235266700");
        assert_eq!(state.items[1].description, "Erdinger Weissbier 0.5L");
        assert_eq!(state.items[1].amount, 1);
        assert_eq!(state.items[1].price, dec!(1.2));
        assert!(!state.show_sale_confirmation);

        let _ = update(&mut state, Message::Pay);
        assert_eq!(state.user, None);
        assert_eq!(state.items.len(), 0);
        assert!(state.show_sale_confirmation);
    }
}
