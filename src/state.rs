use crate::config::Config;
use iced::keyboard::key::Named;
use iced::keyboard::Key;
use std::collections::HashMap;

pub struct State {
    #[allow(dead_code)]
    pub config: Config,

    pub articles: HashMap<String, Article>,
    pub users: HashMap<String, String>,

    pub user: Option<String>,
    pub input: String,
    pub items: Vec<Item>,
    /// Show a confirmation screen until this timer runs out
    pub sale_confirmation_timer: u8,
}

impl State {
    pub fn from_config(config: Config) -> State {
        let articles = vec![
            Article {
                ean: "3800235265659".to_string(),
                description: "Gloriette Cola Mix".to_string(),
                price: 0.9,
            },
            Article {
                ean: "x001wfi0uh".to_string(),
                description: "Bratwurst".to_string(),
                price: 1.5,
            },
            Article {
                ean: "3800235266700".to_string(),
                description: "Erdinger Weissbier 0.5L".to_string(),
                price: 1.2,
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
            sale_confirmation_timer: 0,
        }
    }
}

#[derive(Debug)]
pub struct Article {
    pub ean: String,
    pub description: String,
    pub price: f32,
}

#[derive(Debug, Clone)]
pub struct Item {
    pub ean: String,
    pub amount: u16,
    pub description: String,
    pub price: f32,
}

impl Item {
    pub fn total(&self) -> f32 {
        self.amount as f32 * self.price
    }
}

#[derive(Debug, Clone)]
pub enum Message {
    KeyPress(Key),
    Pay,
    Cancel,
    DecreaseSaleConfirmationTimer,
}

pub fn update(state: &mut State, message: Message) {
    match message {
        Message::KeyPress(Key::Character(c)) => {
            state.input.push_str(c.as_str());
            state.sale_confirmation_timer = 0;
        }
        Message::KeyPress(Key::Named(Named::Enter)) => {
            if state.user.is_some() {
                if let Some(article) = state.articles.get(&state.input) {
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
                                price: article.price,
                            });
                        });
                }
            } else {
                if state.users.contains_key(&state.input) {
                    state.user = Some(state.input.clone());
                }
            }

            state.input.clear();
            state.sale_confirmation_timer = 0;
        }
        Message::Pay => {
            state.user = None;
            state.items.clear();
            state.sale_confirmation_timer = 3;
        }
        Message::Cancel => {
            state.user = None;
            state.items.clear();
        }
        Message::DecreaseSaleConfirmationTimer => {
            state.sale_confirmation_timer = state.sale_confirmation_timer.saturating_sub(1);
        }
        _ => {}
    }
}
